//! The commands the audio engine pages call: status, start, stop, and the
//! per-model spec and options.
//!
//! Install and uninstall live beside the other engines in
//! `audio/engines/commands.rs`; this file is only the runtime's face and the
//! settings door, because exactly what `ensure_started` does is about the
//! server's life rather than about where its files live.

use std::sync::Arc;

use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, State};

use super::families;
use crate::database::Database;

/// One of the two engines, from the key the frontend holds.
fn parse(key: &str) -> Result<super::Purpose, String> {
    super::Purpose::from_key(key).ok_or_else(|| format!("Unknown audio purpose: {key}"))
}

/// One engine's status; `purpose` is `speech` or `wake`.
#[tauri::command]
pub async fn audio_status(purpose: String) -> Result<super::Status, String> {
    Ok(super::status(parse(&purpose)?).await)
}

/// Starts the server, reporting the status a start produced.
///
/// The config is rebuilt from the folder before anything is planned, so the
/// start always describes what is installed *now*. The event is what opens the
/// logs panel here, on an ask rather than on a voice's first use: an explicit
/// toggle wants the output visible, and a server that was already up (a restart
/// of the app beside an orphaned process, say) would never otherwise arrive.
#[tauri::command]
pub async fn audio_start(
    app: AppHandle,
    database: State<'_, Arc<Database>>,
    purpose: String,
) -> Result<super::Status, String> {
    let purpose = parse(&purpose)?;
    super::ensure_started_for(purpose, &app, &database).await?;
    let _ = app.emit(
        "audio-server-state",
        json!({ "running": true, "purpose": purpose.key() }),
    );
    Ok(super::status(purpose).await)
}

/// Stops one engine, if it is held.
///
/// The wake word has to go first. It asks its engine for a window every second,
/// and the next window would start the server straight back -- which is why
/// pressing stop used to look like a restart. The record is corrected with it,
/// so the Wake word page's switch agrees with what happened.
#[tauri::command]
pub async fn audio_stop(
    app: AppHandle,
    database: State<'_, Arc<Database>>,
    purpose: String,
) -> Result<super::Status, String> {
    let purpose = parse(&purpose)?;
    if purpose == super::Purpose::Wake && crate::audio::wake::running() {
        crate::audio::wake::stop();
        let mut settings = crate::audio::wake::settings(&database);
        if settings.enabled {
            settings.enabled = false;
            let _ = crate::audio::wake::save(&database, &settings);
        }
    }
    super::stop(purpose, &app).await;
    Ok(super::status(purpose).await)
}

/// One engine's output, as the Logs panel reads it.
///
/// The same shape `local_model_logs` answers with, under that purpose's session
/// `id` -- so the panel consumes both servers with one drawing path.
#[tauri::command]
pub async fn audio_logs(purpose: String) -> Result<Vec<String>, String> {
    Ok(super::logs(parse(&purpose)?))
}

/// What the audio models page draws: name, family, task, size per model.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub path: String,
    pub name: String,
    pub family: String,
    pub task: String,
    pub size: u64,
}

fn rows() -> Vec<Row> {
    families::installed()
        .into_iter()
        .map(|model| Row {
            path: model.path.display().to_string(),
            name: model.name,
            family: model.family,
            task: match model.task {
                Some(families::Task::Tts) => "tts".to_string(),
                Some(families::Task::Asr) => "asr".to_string(),
                Some(families::Task::Wake) => "wake".to_string(),
                None => "unknown".to_string(),
            },
            size: model.size,
        })
        .collect()
}

#[tauri::command]
pub async fn audio_models() -> Result<Vec<Row>, String> {
    Ok(rows())
}

/// Model path → spec/stored lookup, once per command.
///
/// Two commands ask "is this file still installed", so the check lives in one
/// helper rather than an `ok_or_else` at both sites.
fn resolved(path: &str) -> Result<families::Model, String> {
    families::installed()
        .into_iter()
        .find(|model| model.path.display().to_string() == path)
        .ok_or_else(|| "That model is no longer on disk.".to_string())
}

/// One option as a settings row draws it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionView {
    /// The section the option belongs to: `request` or `load` -- the second
    /// means restarting the engine picks it up, which the row's hint says.
    pub section: String,
    pub name: String,
    pub kind: String,
    pub description: String,
    pub required: bool,
    pub default: Option<Value>,
    /// The choices an enum offers, empty when this is not a choice.
    pub values: Vec<String>,
}

/// What a model's settings page is built from.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpecView {
    /// The model's path, which is what the page saves against.
    pub path: String,
    pub name: String,
    pub family: String,
    /// The family's display name from the spec, when one is shipped.
    pub display_name: Option<String>,
    /// The tasks the family answers, from the spec (`tts`, `clone`, `design`,
    /// ...) -- what a reader needs to know without opening the docs.
    pub tasks: Vec<String>,
    /// `tts` / `asr` / `unknown`, as the list page carries it.
    pub task: String,
    /// Capability tags per task, from the spec.
    pub capabilities: Value,
    pub options: Vec<OptionView>,
    /// What the user has stored for this model, whole.
    pub values: Value,
}

#[tauri::command]
pub async fn audio_model_spec(
    database: tauri::State<'_, Arc<Database>>,
    path: String,
) -> Result<SpecView, String> {
    let model = resolved(&path)?;
    let spec = super::specs::for_family(&model.family);
    let mut options: Vec<OptionView> = Vec::new();
    if let Some(spec) = spec.as_deref() {
        for (section, list) in [
            ("request", &spec.request),
            ("load", &spec.load),
            ("session", &spec.session),
        ] {
            for option in list {
                options.push(OptionView {
                    section: section.to_string(),
                    name: option.name.clone(),
                    kind: match option.kind {
                        super::specs::OptionKind::Text => "text",
                        super::specs::OptionKind::Integer => "int",
                        super::specs::OptionKind::Number => "float",
                        super::specs::OptionKind::Flag => "bool",
                        super::specs::OptionKind::Choice => "enum",
                    }
                    .to_string(),
                    description: option.description.clone(),
                    required: option.required,
                    default: option.default.clone(),
                    values: option.values.clone(),
                });
            }
        }
    }
    Ok(SpecView {
        path: model.path.display().to_string(),
        name: model.name.clone(),
        family: model.family.clone(),
        display_name: spec.as_deref().map(|spec| spec.display_name.clone()),
        tasks: spec
            .as_deref()
            .map(|spec| spec.tasks.clone())
            .unwrap_or_default(),
        task: match model.task {
            Some(families::Task::Tts) => "tts".to_string(),
            Some(families::Task::Asr) => "asr".to_string(),
            Some(families::Task::Wake) => "wake".to_string(),
            None => "unknown".to_string(),
        },
        capabilities: spec
            .as_deref()
            .map(|spec| json!(spec.capabilities))
            .unwrap_or_else(|| json!({})),
        options,
        values: super::settings::stored(&database, &model.path.display().to_string())
            .unwrap_or_else(|| json!({})),
    })
}

/// Saves one model's options, whole.
///
/// The caller sends one merged object of every option's current value; an
/// empty object clears the record, which is what "reset to defaults" saves.
#[tauri::command]
pub async fn audio_model_save_options(
    database: tauri::State<'_, Arc<Database>>,
    path: String,
    values: Value,
) -> Result<(), String> {
    resolved(&path)?;
    super::settings::save(&database, &path, &values)
}
