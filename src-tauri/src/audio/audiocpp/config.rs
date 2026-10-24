//! The server config, written per launch.
//!
//! `audiocpp_server` is driven by a JSON file: which models it may load, under
//! which family, for which task, at what address and backend. The file is built
//! from what is installed the moment the server starts, so a model added while
//! it is not running is picked up on the next start, and a server restart after
//! installing one is the shortest path to a config that tells the truth.
//!
//! `lazy_load` is load-bearing rather than a nicety: audio.cpp keeps a model
//! resident once used, and several of its TTS families are as heavy as an LLM.
//! With lazy loading a server cost nothing at start-up, and with
//! `max_loaded_models: 1` it also evicts, so a speech model does not sit on the
//! GPU for an app that may be trying to run a local LLM beside it.

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::app_paths::tmp_root;
use crate::database::Database;

use super::families::{Model, Task};
use super::{settings, specs};

/// The address and shape the server is started with.
const PORT: u32 = 43126;

/// The server's backend. Vulkan, matching the whisper build this app installs
/// already: the AMD story is the whole reason the Vulkan build exists.
const BACKEND: &str = "vulkan";

/// Where the config is written: the app's own scratch space.
pub fn config_path() -> PathBuf {
    tmp_root().join("audio").join("server.json")
}

/// The JSON one launch is handed.
///
/// Each known model is one entry; the `load` options the user stored for a
/// model (from its family's spec) ride on the entry, because load options are
/// model-level rather than per message.
pub fn build(models: &[Model], database: &Database) -> Value {
    let session_only = |family: &str| {
        settings::espeak_trait(database, family) == Some(settings::EspeakTrait::Session)
    };
    build_from(
        models,
        &|path| settings::stored(database, path),
        &|family| Some(session_only(family)),
    )
}

/// The build, with the per-model stored lookup and the families' learned
/// traits taken as functions -- so the tests can stand in for the settings
/// record and the negotiation memory without a database file.
fn build_from(
    models: &[Model],
    stored_for: &dyn Fn(&str) -> Option<Value>,
    espeak_for: &dyn Fn(&str) -> Option<bool>,
) -> Value {
    let mut entries: Vec<Value> = Vec::new();
    let mut used: Vec<String> = Vec::new();
    for model in models {
        let Some(task) = model.task else {
            // An unknown family cannot be configured -- refusing by name happens
            // wherever the user tries to use it, so the config skips it rather
            // than serving a model the server would fail to load.
            continue;
        };
        let mut id = model.slug();
        if id.is_empty() {
            id = "model".to_string();
        }
        // Two files may slug the same -- a stem shared across sub-folders -- so
        // the second one carries a suffix rather than silently shadowing the
        // first.
        let mut index = 2;
        while used.contains(&id) {
            id = format!("{}-{index}", model.slug());
            index += 1;
        }
        used.push(id.clone());

        let path = model.path.display().to_string();
        let stored = stored_for(&path);
        let stored_object = stored.as_ref().and_then(Value::as_object);
        let spec = specs::for_family(&model.family);
        let load = settings::load_values(spec.as_deref(), stored_object);
        // Session options: the user's own first, then -- only when the family
        // wants eSpeak's paths and the user has not set them -- the machine's
        // detected install. Who "wants" is two sources: the spec naming the
        // option, or a trait the runtime *learned* from a previous refusal
        // (Kokoro's spec is silent while its runtime needs them). No source,
        // no key -- an espeak-free family carries nothing to refuse.
        let wants_espeak = settings::spec_takes_session(spec.as_deref(), "espeak_library_path")
            || espeak_for(&model.family) == Some(true);
        let mut session = settings::session_values(spec.as_deref(), &model.family, stored_object);
        if wants_espeak
            && !stored_object.is_some_and(|entry| entry.contains_key("espeak_library_path"))
        {
            if let Some((library, data)) = espeak_paths() {
                session.insert(
                    format!("{}.espeak_library_path", model.family),
                    Value::String(library.display().to_string()),
                );
                session.insert(
                    format!("{}.espeak_data_path", model.family),
                    Value::String(data.display().to_string()),
                );
            }
        }

        let mut entry = json!({
            "id": id,
            "family": model.family,
            "path": path,
            "task": task_name(task),
            "mode": "offline",
        });
        if load.as_object().is_some_and(|options| !options.is_empty()) {
            entry["load_options"] = load;
        }
        if !session.is_empty() {
            entry["session_options"] = Value::Object(session);
        }
        entries.push(entry);
    }

    json!({
        "host": "127.0.0.1",
        "port": PORT,
        "backend": BACKEND,
        "lazy_load": true,
        "max_loaded_models": 1,
        "models": entries,
    })
}

/// The config as text, with the folder it lives in made first.
pub fn write(models: &[Model], database: &Database) -> Result<(), String> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(&build(models, database))
        .map_err(|error| format!("The server configuration could not be written: {error}"))?;
    std::fs::write(&path, text)
        .map_err(|error| format!("Could not write {}: {error}", path.display()))
}

/// The launch arguments: this config and nothing else.
///
/// `--config` must arrive as two arguments, not `--config=<path>`: the server
/// reports "missing required --config argument" for the `=` form regardless of
/// what it carries -- confirmed against the real binary -- which is exactly how
/// the first in-app runs died before the server ever listened. No `--ui`, so
/// the embedded WebUI and its model-management surface stay off; the app owns
/// what is allowed, and a second admin surface on the same port would be two
/// answers to one question.
///
/// `--log` is not optional here, though it is an option in the server's own
/// guide: the logs panel is how a failed model load gets diagnosed, and a
/// server started without it prints nothing -- which is precisely what made
/// the panel read "Waiting for the audio engine…" forever while the first
/// request died somewhere this app could not see.
pub fn args() -> Vec<String> {
    vec![
        "--config".to_string(),
        config_path().display().to_string(),
        "--log".to_string(),
    ]
}

/// The working directory the server runs in.
///
/// The engine's own install root, *not* the scratch folder: the server's spec
/// lookup walks the working directory for `model_specs/<family>.json`, and the
/// release ships those specs beside the binary. From the scratch folder the
/// lookup would never find them, and a multi-file family would fail to load
/// with everything else about the request exactly right.
pub fn working_dir() -> PathBuf {
    crate::audio::engines::audiocpp::installed_dir().unwrap_or_else(tmp_root)
}

/// The eSpeak-ng install the machine carries as `(library, data)`, or `None`
/// when there is none. The adapter reads `espeak_library_path` and
/// `espeak_data_path` when given, and without them probes only where eSpeak
/// itself was configured at build time.
///
/// The app's own install wins -- it is what the Audio engine page puts there,
/// and it lives inside the app so deleting it means deleting the app. The
/// upstream Windows installer into `Program Files\eSpeak NG` (or the
/// user-scoped mirror under `%LOCALAPPDATA%\Programs`) is a fallback.
fn espeak_paths() -> Option<(PathBuf, PathBuf)> {
    if crate::audio::engines::espeak::paths().is_some() {
        return crate::audio::engines::espeak::paths();
    }
    let mut candidates = vec![
        PathBuf::from(r"C:\Program Files\eSpeak NG"),
        PathBuf::from(r"C:\Program Files (x86)\eSpeak NG"),
    ];
    if let Some(local) = dirs_program_files_local() {
        candidates.push(local);
    }
    for folder in candidates {
        let library = folder.join("espeak-ng.dll");
        let data = folder.join("espeak-ng-data");
        if library.is_file() && data.is_dir() {
            return Some((library, data));
        }
    }
    None
}

/// `%LOCALAPPDATA%\Programs\eSpeak NG`, since `%LOCALAPPDATA%` is not
/// available to a call without it.
fn dirs_program_files_local() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .as_deref()
        .map(std::path::Path::new)
        .map(|root| root.join("Programs").join("eSpeak NG"))
}

fn task_name(task: Task) -> &'static str {
    match task {
        Task::Tts => "tts",
        Task::Asr => "asr",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(name: &str, family: &str, task: Task) -> Model {
        Model {
            // A real file carries `.gguf`, and the stem is what the slug is
            // taken from -- the extension is not part of it.
            path: std::path::PathBuf::from("/x/y").join(format!("{name}.gguf")),
            name: format!("{name}.gguf"),
            family: family.to_string(),
            task: Some(task),
            size: 1,
        }
    }

    #[test]
    fn every_known_model_is_configured_once() {
        let models = vec![
            model("Kokoro-82M", "kokoro_tts", Task::Tts),
            model("Kokoro-82M", "kokoro_tts", Task::Tts),
            model("qwen3-asr-0.6b", "qwen3_asr", Task::Asr),
        ];
        let config = build_from(&models, &|_| None, &|_| None);
        let entries = config["models"].as_array().unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0]["id"], "kokoro-82m");
        // The same stem twice does not produce two identical ids.
        assert_eq!(entries[1]["id"], "kokoro-82m-2");
        assert_eq!(entries[2]["id"], "qwen3-asr-0-6b");
    }

    #[test]
    fn an_unknown_family_is_left_out() {
        let mut unknown = model("some-future-model", "unknown", Task::Tts);
        unknown.task = None;
        let config = build_from(&[unknown], &|_| None, &|_| None);
        let entries = config["models"].as_array().unwrap();
        assert_eq!(entries.len(), 0);
    }

    #[test]
    fn stored_load_options_ride_on_the_entry() {
        // The spec is read from the engine's install here and would be None
        // without one, so the loader only sees what the stored record names --
        // which is the point: a stored option the spec does not name never
        // reaches the server config.
        let models = vec![model("Kokoro-82M", "unknown", Task::Tts)];
        let config = build_from(
            &models,
            &|_| Some(serde_json::json!({ "Any": "value" })),
            &|_| None,
        );
        let entry = &config["models"].as_array().unwrap()[0];
        assert!(entry.get("load_options").is_none());
    }
}
