//! What a model can do, as the engine's own spec states it.
//!
//! audio.cpp ships `model_specs/<family>.json` beside the binaries: per family,
//! its tasks, its capabilities (voice design, speaker reference, style
//! control, ...) and the request/load options it takes, typed, with defaults.
//! That file is the single source of truth for what a model can do -- the app
//! renders a model's settings page from it rather than keeping a second copy
//! of a hundred families' option lists.
//!
//! # Why cached
//!
//! The specs are read from the installed engine's directory, and a settings
//! page may ask for the same family repeatedly while its controls render. One
//! parse per family per process is the entire cost; the cache is cleared
//! whenever the engine changes, which is what a restart already is.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};

use serde_json::Value;

/// One option a model takes, typed for the settings page and the request body.
#[derive(Clone, Debug)]
pub struct OptionSpec {
    /// The name the request body (or the server config) carries it under.
    pub name: String,
    pub kind: OptionKind,
    /// The option's own words, which are what a settings row's hint says.
    pub description: String,
    pub default: Option<Value>,
    pub required: bool,
    /// The values a choice offers, filled when `kind` is `Choice`.
    pub values: Vec<String>,
}

/// What shape a value takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OptionKind {
    /// Rendered as a text field; sent as a JSON string.
    Text,
    /// Rendered as a number field; sent as a JSON integer.
    Integer,
    /// Rendered as a number field; sent as a JSON float.
    Number,
    /// Rendered as a toggle; sent as a JSON boolean.
    Flag,
    /// Rendered as a dropdown over `values`; sent as a JSON string.
    Choice,
}

impl OptionKind {
    /// The engine's type string, lowered.
    fn parse(name: &str) -> OptionKind {
        match name.trim().to_ascii_lowercase().as_str() {
            "string" | "text" => OptionKind::Text,
            "int" => OptionKind::Integer,
            "float" | "number" => OptionKind::Number,
            "bool" => OptionKind::Flag,
            "enum" => OptionKind::Choice,
            // Unknown types are still options; a text field is the shape that
            // cannot lose information, and the server validates the value.
            _ => OptionKind::Text,
        }
    }
}

/// One model family's spec, as `audiocpp_cli` would print it.
#[derive(Clone, Debug)]
pub struct FamilySpec {
    pub display_name: String,
    /// The tasks the family answers: `tts`, `clone`, `design`, `asr`, ...
    pub tasks: Vec<String>,
    /// Capability tags per task -- `voice_design`, `speaker_reference`,
    /// `style_control`. The settings page may use them as a hint; the options
    /// below are the actual contract.
    pub capabilities: BTreeMap<String, Vec<String>>,
    /// Request options: sent with every speech or transcription call.
    pub request: Vec<OptionSpec>,
    /// Load options: sent in the server config, so they are model-level, not
    /// per message.
    pub load: Vec<OptionSpec>,
    /// Session options: like load, they live in the server config's entry, but
    /// under the engine's own `session_options` block -- and the keys the
    /// engine resolves are *family-namespaced* (`inflect_v2.espeak_data_path`),
    /// not the bare names the spec lists.
    pub session: Vec<OptionSpec>,
}

/// The folder the installed engine's specs live in.
fn specs_dir() -> Option<PathBuf> {
    crate::audio::engines::audiocpp::installed_dir().map(|dir| dir.join("model_specs"))
}

/// One family's spec, or `None` when the engine does not ship one.
pub fn for_family(family: &str) -> Option<Arc<FamilySpec>> {
    static CACHE: LazyLock<Mutex<BTreeMap<String, Option<Arc<FamilySpec>>>>> =
        LazyLock::new(|| Mutex::new(BTreeMap::new()));
    let mut cache = CACHE.lock().ok()?;
    if let Some(found) = cache.get(family) {
        return found.clone();
    }
    let parsed = read(family);
    cache.insert(family.to_string(), parsed.clone());
    parsed
}

/// Parses one spec file from the engine's install.
fn read(family: &str) -> Option<Arc<FamilySpec>> {
    let directory = specs_dir()?;
    let path = directory.join(format!("{family}.json"));
    let text = std::fs::read_to_string(path).ok()?;
    let json: Value = serde_json::from_str(&text).ok()?;
    let id = json.get("family").and_then(Value::as_str)?.to_string();
    if id != family {
        return None;
    }
    Some(Arc::new(FamilySpec {
        display_name: json
            .get("display_name")
            .and_then(Value::as_str)
            .unwrap_or(family)
            .to_string(),
        tasks: string_list(&json["tasks"]),
        capabilities: json
            .get("capabilities")
            .and_then(Value::as_object)
            .map(|capabilities| {
                capabilities
                    .iter()
                    .map(|(task, tags)| (task.clone(), string_list(tags)))
                    .collect()
            })
            .unwrap_or_default(),
        request: json
            .get("options")
            .and_then(|options| options.get("request"))
            .map(parse_options)
            .unwrap_or_default(),
        load: json
            .get("options")
            .and_then(|options| options.get("load"))
            .map(parse_options)
            .unwrap_or_default(),
        session: json
            .get("options")
            .and_then(|options| options.get("session"))
            .map(parse_options)
            .unwrap_or_default(),
    }))
}

impl FamilySpec {
    /// Whether the spec's session section declares one named option -- the
    /// gate the config's eSpeak injection reads, so a family that does not
    /// take eSpeak paths never receives them and the server has nothing to
    /// call "unknown".
    pub fn has_session(&self, name: &str) -> bool {
        self.session.iter().any(|option| option.name == name)
    }
}

/// Every string in one JSON array, tolerant of a malformed entry.
fn string_list(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The typed options one section of the spec names.
fn parse_options(value: &Value) -> Vec<OptionSpec> {
    let Some(options) = value.as_array() else {
        return Vec::new();
    };
    options
        .iter()
        .filter_map(|option| {
            Some(OptionSpec {
                name: option.get("name")?.as_str()?.to_string(),
                kind: OptionKind::parse(
                    option
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("string"),
                ),
                description: option
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                default: option.get("default").cloned().filter(|v| !v.is_null()),
                required: option
                    .get("required")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                values: string_list(&option["values"]),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_engine_types_map_to_the_shapes_the_page_draws() {
        assert_eq!(OptionKind::parse("string"), OptionKind::Text);
        assert_eq!(OptionKind::parse("int"), OptionKind::Integer);
        assert_eq!(OptionKind::parse("float"), OptionKind::Number);
        assert_eq!(OptionKind::parse("bool"), OptionKind::Flag);
        assert_eq!(OptionKind::parse("enum"), OptionKind::Choice);
        // A type this app has not seen is still renderable -- as text, which the
        // server validates at its end.
        assert_eq!(OptionKind::parse("sequence"), OptionKind::Text);
    }
}
