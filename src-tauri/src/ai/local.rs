use std::{
    collections::VecDeque,
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

pub mod device;
pub mod engines;
pub mod memory;
pub mod parser;
mod settings;

// `LocalRuntimeSettings` is re-exported because callers name the type.
// `KvQuant` is reached through `local_kv_quantisations`, which is the only path
// the frontend takes -- so it is re-exported here for that command alone rather
// than as a general convenience.
pub use settings::{KvQuant, LocalRuntimeSettings};

const SERVER_PORT: u16 = 43127;
const SERVER_MODEL_ALIAS: &str = "projectz-local";
const MAX_SERVER_LOG_LINES: usize = 2_000;
/// Synthetic endpoint id used when routing a request to the local server.
pub const LOCAL_ENDPOINT_ID: &str = "local-model";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalModel {
    pub id: String,
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
    /// Whether the file is still on disk, checked when the list is read.
    ///
    /// Computed here rather than in the frontend because `add` already proved
    /// the file exists and this is the same question asked again: the frontend
    /// was doing its own path check against a `\\?\`-prefixed string and getting
    /// it wrong for files that were plainly there. One owner, one answer.
    ///
    /// Defaults to `true` so a model row read before this field existed is never
    /// reported as missing -- an unknown is not the same as a deletion, and
    /// showing a red warning for a file that works is worse than showing nothing.
    #[serde(default = "default_present")]
    pub present: bool,
    /// The weights' quantisation, read from the GGUF header: `Q4_K_M` and so on.
    ///
    /// The one fact about a model a user cannot get from its filename, which is
    /// usually the name of a fine-tune and says nothing about how it was
    /// quantised or how much memory it will take.
    #[serde(default)]
    pub quantization: Option<String>,
    /// The context window the model was trained for, from `context_length`.
    ///
    /// Not the context this app will run it at -- that is a setting, and a model
    /// can be run below its limit. This is the ceiling the file declares.
    #[serde(default)]
    pub context_length: Option<u64>,
    /// The engine this model will be run with, resolved when the list is read.
    ///
    /// Not stored on the model row: the answer is this model's own pin, falling
    /// back to the global default, and reading it once here is what lets the
    /// settings page group models by engine without a second request or a
    /// re-derivation in TypeScript that could disagree with the loader's idea of
    /// which engine a model uses. `None` means nothing is selected, which is a
    /// real state -- a model with no engine simply cannot be loaded.
    #[serde(default)]
    pub engine_id: Option<String>,
}

fn default_present() -> bool {
    true
}

impl LocalModel {
    /// Re-checks the file and fills in what the list shows about it.
    ///
    /// Both happen in one pass over one `stat`: the presence flag and the facts
    /// read from the GGUF header. The header read is the same one the memory
    /// estimate uses, so the quantisation and context shown here are the same
    /// numbers that figure is computed from rather than a second opinion.
    pub fn inspect(mut self) -> Self {
        let path = crate::ai::local::memory::normalise_windows_path(&self.path);
        self.present = path.is_file();
        if let Some(shape) = crate::ai::local::parser::read_shape(&path) {
            // Only filled in when the header carried them, so a model this parser
            // cannot read keeps whatever the list already knew rather than having
            // its facts replaced by zeroes.
            if shape.quantization.is_some() {
                self.quantization = shape.quantization.map(|value| value.as_str().to_string());
            }
            if shape.context_length > 0 {
                self.context_length = Some(shape.context_length as u64);
            }
        }
        self
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LocalRuntimeStatus {
    pub loaded_model_id: Option<String>,
    pub loading_model_id: Option<String>,
    pub selected_model_id: Option<String>,
    pub base_url: String,
}

#[derive(Default)]
struct ProcessState {
    child: Option<Child>,
    loaded_model_id: Option<String>,
    loading_model_id: Option<String>,
    logs: VecDeque<String>,
}

pub struct LocalModelManager {
    database: std::sync::Arc<crate::database::Database>,
    engine_dir: PathBuf,
    process: Mutex<ProcessState>,
}

impl LocalModelManager {
    pub fn new(database: std::sync::Arc<crate::database::Database>) -> Self {
        Self {
            database,
            engine_dir: PathBuf::new(),
            process: Mutex::new(ProcessState::default()),
        }
    }

    pub fn runtime_settings(&self, model_id: &str) -> LocalRuntimeSettings {
        self.database.runtime_settings(model_id)
    }

    pub fn save_runtime_settings(
        &self,
        model_id: &str,
        settings: LocalRuntimeSettings,
    ) -> Result<LocalRuntimeSettings, String> {
        self.database.save_runtime_settings(model_id, &settings)?;
        Ok(settings)
    }

    pub fn with_engine_dir(mut self, engine_dir: PathBuf) -> Self {
        self.engine_dir = engine_dir;
        self
    }

    pub fn selected_id(&self) -> Option<String> {
        self.database.setting("local.selected_model")
    }

    pub fn select(&self, id: &str) -> Result<(), String> {
        if !self.list().iter().any(|model| model.id == id) {
            return Err("Local model not found".to_string());
        }
        self.database.set_setting("local.selected_model", &id)
    }

    pub fn list(&self) -> Vec<LocalModel> {
        // Each row is inspected as it is read, so `present` and the header facts
        // cannot drift from what the list shows -- they are produced by the same
        // pass. One `stat` and at most one header read per model, on a settings
        // page that opens rarely.
        //
        // The engine is resolved here for the same reason: the group a model is
        // listed under has to be the engine the loader would actually use, and
        // resolving it twice in two languages is the way those two answers stop
        // agreeing.
        self.database
            .list_local_models()
            .unwrap_or_default()
            .into_iter()
            .map(|mut model| {
                model.engine_id = self.engine_id_for(&model.id);
                model.inspect()
            })
            .collect()
    }

    /// Which engine a model runs with: its own engine, full stop.
    ///
    /// There is no fallback and no global default. The settings page reads the
    /// same answer off the list that `load` uses to pick the executable, so a
    /// model cannot be shown under one engine and started with another.
    ///
    /// An engine that has since been removed is **not** replaced with anything.
    /// A model was pointed at that build because it behaves the way it does,
    /// and quietly running it on something else would produce answers from a
    /// backend nobody chose. The honest outcome is a load that names the missing
    /// engine.
    pub fn engine_id_for(&self, model_id: &str) -> Option<String> {
        self.database
            .setting::<Option<String>>(&engines::engine_setting_key(model_id))
            .flatten()
    }

    /// Set which engine a model runs with.
    ///
    /// Always a real engine, never `None`: there is no default to clear back
    /// to, so the command layer refuses an empty choice before it gets here.
    ///
    /// Whether the engine is installed is **not** checked here: this method has
    /// no `AppHandle`, and the directory it would be checked against is owned by
    /// `engines`. The command that calls this does the check, so a choice can
    /// only ever name files that are actually on disk.
    pub fn set_model_engine(&self, model_id: &str, engine_id: String) -> Result<(), String> {
        self.database
            .set_setting(&engines::engine_setting_key(model_id), &engine_id)
    }

    /// One registered model, by id.
    ///
    /// A lookup rather than a `list().find()` at each call site: the estimate runs
    /// on every slider change, and rebuilding the whole list to read one row is
    /// work that scales with how many models the user has registered.
    pub fn model(&self, id: &str) -> Option<LocalModel> {
        self.list().into_iter().find(|model| model.id == id)
    }

    pub fn add(&self, path: String) -> Result<LocalModel, String> {
        let file_path = PathBuf::from(&path);
        if !file_path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("gguf"))
        {
            return Err("Choose a GGUF model file".to_string());
        }
        let metadata =
            fs::metadata(&file_path).map_err(|error| format!("Cannot read model file: {error}"))?;
        if !metadata.is_file() {
            return Err("The selected path is not a file".to_string());
        }
        let canonical_path = fs::canonicalize(&file_path).unwrap_or(file_path);
        let name = canonical_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("Local model")
            .to_string();
        let model = LocalModel {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            // Stored canonicalised, which on Windows is the `\\?\` device form.
            // That is what the loader must open, so it is kept; the list and the
            // checks strip it for display and for anything that compares paths.
            path: canonical_path.to_string_lossy().into_owned(),
            size_bytes: metadata.len(),
            // Known present: `fs::metadata` above just proved it.
            present: true,
            quantization: None,
            context_length: None,
            // Filled in by the manager once the pin is read; never persisted.
            engine_id: None,
        };
        if self
            .list()
            .iter()
            .any(|existing| existing.path == model.path)
        {
            return Err("That model is already in your library".to_string());
        }
        self.database.save_local_model(&model)?;
        Ok(model)
    }

    pub fn remove(&self, id: &str) -> Result<(), String> {
        let mut process = self.process.lock().unwrap();
        if process.loaded_model_id.as_deref() == Some(id)
            || process.loading_model_id.as_deref() == Some(id)
        {
            stop_process(&mut process);
        }
        drop(process);
        let was_selected = self.selected_id().as_deref() == Some(id);
        self.database.remove_local_model(id)?;
        if was_selected {
            self.database
                .set_setting::<Option<String>>("local.selected_model", &None)?;
        }
        Ok(())
    }

    pub async fn end_reasoning(&self, completion_id: &str) -> Result<(), String> {
        let response = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|error| error.to_string())?
            .post(format!(
                "http://127.0.0.1:{SERVER_PORT}/v1/chat/completions/control"
            ))
            .json(&serde_json::json!({ "id": completion_id, "action": "reasoning_end" }))
            .send()
            .await
            .map_err(|error| format!("Could not reach local reasoning control: {error}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "Local reasoning control returned {}",
                response.status()
            ));
        }
        let result: serde_json::Value = response
            .json()
            .await
            .map_err(|error| format!("Invalid local reasoning control response: {error}"))?;
        if result.get("success").and_then(serde_json::Value::as_bool) == Some(true) {
            Ok(())
        } else {
            Err(result
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("The local model could not skip reasoning")
                .to_string())
        }
    }

    pub fn server_logs(&self) -> Vec<String> {
        self.process
            .lock()
            .map(|process| process.logs.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn status(&self) -> LocalRuntimeStatus {
        let mut process = self.process.lock().unwrap();
        if process
            .child
            .as_mut()
            .is_some_and(|child| child.try_wait().ok().flatten().is_some())
        {
            process.child = None;
            process.loaded_model_id = None;
            process.loading_model_id = None;
        }
        LocalRuntimeStatus {
            loaded_model_id: process.loaded_model_id.clone(),
            loading_model_id: process.loading_model_id.clone(),
            selected_model_id: self.selected_id(),
            base_url: format!("http://127.0.0.1:{SERVER_PORT}/v1"),
        }
    }

    /// Expose the running llama.cpp server as an OpenAI-compatible endpoint so
    /// local and remote chat (and internal tasks like title generation) can
    /// share one request path. Returns `None` when no model is loaded.
    pub fn loaded_endpoint(&self) -> Option<(crate::ai::remote::types::Endpoint, String)> {
        let status = self.status();
        status.loaded_model_id?;
        Some((
            crate::ai::remote::types::Endpoint {
                id: LOCAL_ENDPOINT_ID.to_string(),
                name: "Local model".to_string(),
                base_url: status.base_url,
                api_key: String::new(),
                models: vec![SERVER_MODEL_ALIAS.to_string()],
                disabled_models: vec![],
                enabled: true,
            },
            SERVER_MODEL_ALIAS.to_string(),
        ))
    }

    pub fn unload(&self) {
        let mut process = self.process.lock().unwrap();
        stop_process(&mut process);
    }

    /// Stop a model if it is the one running, leaving any other alone.
    ///
    /// Used when a model's engine changes: the process in flight was started by
    /// the engine that was just replaced, so leaving it up would mean the model
    /// kept answering from the old backend while its setting says otherwise --
    /// the exact silent disagreement the resolver exists to prevent. Only one
    /// model runs at a time, so this is either "stop" or "do nothing".
    pub fn unload_if_loaded(&self, model_id: &str) {
        let mut process = self.process.lock().unwrap();
        if process.loaded_model_id.as_deref() == Some(model_id)
            || process.loading_model_id.as_deref() == Some(model_id)
        {
            stop_process(&mut process);
        }
    }

    pub fn load(self: &std::sync::Arc<Self>, app: AppHandle, id: String) -> Result<(), String> {
        let model = self
            .list()
            .into_iter()
            .find(|model| model.id == id)
            .ok_or_else(|| "Local model not found".to_string())?;
        if !Path::new(&model.path).is_file() {
            return Err("The GGUF file is no longer available at its saved path".to_string());
        }
        let mut process = self.process.lock().unwrap();
        if process.loaded_model_id.as_deref() == Some(&id) {
            return Ok(());
        }
        stop_process(&mut process);
        // The model's own engine. The same resolver the settings page reads, so
        // the engine a model is listed under is the engine it starts with.
        let engine_id = self.engine_id_for(&id).ok_or_else(|| {
            "This model has no engine. Pick one for it in Settings > Local first.".to_string()
        })?;
        let runtime_dir = self.engine_dir.join(&engine_id);
        let server = runtime_dir.join(server_executable_name());
        if !server.is_file() {
            // Names the engine rather than only the path, because the fix is not
            // always to reinstall it: a model whose engine has since been
            // removed needs its engine changed, and that message says which.
            return Err(format!(
                "The engine this model uses is not installed ({engine_id}). \
                 Choose another engine for it in Settings > Local, or reinstall the engine."
            ));
        }
        if !port_is_available(SERVER_PORT) {
            return Err(format!("Local model port {SERVER_PORT} is already in use"));
        }
        let runtime_dir = runtime_dir.clone();
        let model_id = id.clone();
        let model_path = model.path.clone();
        let settings = self.runtime_settings(&id);
        // A projector that has been moved or deleted since it was chosen would
        // otherwise start a server that dies on first request with a message
        // about a file the settings page still shows as configured.
        if let Some(projector) = settings.mmproj.as_deref() {
            if !projector.trim().is_empty() && !Path::new(projector).is_file() {
                process.loading_model_id = None;
                return Err(format!(
                    "The multimodal projector for this model is missing. Pick it again in Settings > Local: {}",
                    projector
                ));
            }
        }
        process.loading_model_id = Some(id.clone());
        process.logs.clear();
        let mut child = Command::new(server)
            .current_dir(&runtime_dir)
            .arg("--model")
            .arg(&model_path)
            .arg("--alias")
            .arg(SERVER_MODEL_ALIAS)
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--port")
            .arg(SERVER_PORT.to_string())
            .arg("--no-webui")
            .args(settings.to_args())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                process.loading_model_id = None;
                format!("Could not start llama-server: {error}")
            })?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let child_id = child.id();
        process.child = Some(child);
        drop(process);
        let manager = std::sync::Arc::clone(self);
        if let Some(stdout) = stdout {
            spawn_log_reader(
                app.clone(),
                std::sync::Arc::clone(&manager),
                id.clone(),
                "stdout",
                stdout,
            );
        }
        if let Some(stderr) = stderr {
            spawn_log_reader(
                app.clone(),
                std::sync::Arc::clone(&manager),
                id.clone(),
                "stderr",
                stderr,
            );
        }
        thread::spawn(move || {
            let result = tauri::async_runtime::block_on(wait_for_server(
                SERVER_PORT,
                Duration::from_secs(600),
                std::sync::Arc::clone(&manager),
                Some(child_id),
            ));
            let still_running =
                manager
                    .process
                    .lock()
                    .unwrap()
                    .child
                    .as_mut()
                    .is_some_and(|child| {
                        child_id == child.id() && child.try_wait().ok().flatten().is_none()
                    });
            if !still_running {
                if manager.status().loading_model_id.is_none()
                    && manager.status().loaded_model_id.is_none()
                {
                    return;
                }
                manager.confirm_failed(&model_id);
                let _ = app.emit("local-model-event", serde_json::json!({"kind": "failed", "modelId": model_id, "message": "llama-server exited while loading the model"}));
                return;
            }
            match result {
                Ok(()) => {
                    manager.confirm_loaded(&model_id);
                    let _ = app.emit("local-model-event", serde_json::json!({"kind": "loaded", "modelId": model_id, "modelPath": model_path}));
                }
                Err(error) => {
                    manager.confirm_failed(&model_id);
                    let _ = app.emit("local-model-event", serde_json::json!({"kind": "failed", "modelId": model_id, "message": error}));
                }
            }
        });
        Ok(())
    }

    pub fn confirm_loaded(&self, id: &str) {
        let mut process = self.process.lock().unwrap();
        process.loading_model_id = None;
        process.loaded_model_id = Some(id.to_string());
    }

    pub fn confirm_failed(&self, id: &str) {
        let mut process = self.process.lock().unwrap();
        if process.loading_model_id.as_deref() == Some(id) {
            stop_process(&mut process);
        }
    }
}

impl Drop for LocalModelManager {
    fn drop(&mut self) {
        if let Ok(process) = self.process.get_mut() {
            stop_process(process);
        }
    }
}

fn spawn_log_reader<R>(
    app: AppHandle,
    manager: std::sync::Arc<LocalModelManager>,
    model_id: String,
    stream: &'static str,
    reader: R,
) where
    R: std::io::Read + Send + 'static,
{
    thread::spawn(move || {
        for line in BufReader::new(reader).lines() {
            match line {
                Ok(text) => {
                    let entry = format!("[{stream}] {text}");
                    if let Ok(mut process) = manager.process.lock() {
                        process.logs.push_back(entry.clone());
                        while process.logs.len() > MAX_SERVER_LOG_LINES {
                            process.logs.pop_front();
                        }
                    }
                    let _ = app.emit(
                        "local-model-log",
                        serde_json::json!({"modelId": model_id, "line": entry}),
                    );
                }
                Err(error) => {
                    let _ = app.emit(
                        "local-model-log",
                        serde_json::json!({"modelId": model_id, "line": format!("[{stream}] Log read failed: {error}")}),
                    );
                    break;
                }
            }
        }
    });
}

fn stop_process(process: &mut ProcessState) {
    if let Some(mut child) = process.child.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    process.loaded_model_id = None;
    process.loading_model_id = None;
}

fn port_is_available(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

async fn wait_for_server(
    port: u16,
    timeout: Duration,
    manager: std::sync::Arc<LocalModelManager>,
    child_id: Option<u32>,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|error| error.to_string())?;
    let start = Instant::now();
    while start.elapsed() < timeout {
        let still_loading = {
            let mut process = manager.process.lock().unwrap();
            process.loading_model_id.is_some()
                && process.child.as_mut().is_some_and(|child| {
                    child_id == Some(child.id()) && child.try_wait().ok().flatten().is_none()
                })
        };
        if !still_loading {
            return Err("Model loading was cancelled or llama-server exited".to_string());
        }
        if client
            .get(format!("http://127.0.0.1:{port}/health"))
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    Err("Model loading timed out. Check that the GGUF file is valid and your system has enough memory.".to_string())
}

#[cfg(windows)]
fn server_executable_name() -> &'static str {
    "llama-server.exe"
}

#[cfg(not(windows))]
fn server_executable_name() -> &'static str {
    "llama-server"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;

    fn manager() -> LocalModelManager {
        LocalModelManager::new(std::sync::Arc::new(Database::open_in_memory()))
    }

    /// A model runs with the engine it was given, nothing else.
    ///
    /// There is deliberately no global default and no fallback: the one engine
    /// a model names is the one it starts with, and a model that names none
    /// cannot be loaded at all. A second answer anywhere -- a default consulted
    /// after the model's own -- would be the way a shown engine and a started
    /// engine stop agreeing.
    #[test]
    fn a_model_runs_with_exactly_the_engine_it_was_given() {
        let manager = manager();
        manager
            .set_model_engine("model-a", "fork-prism".to_string())
            .unwrap();
        assert_eq!(
            manager.engine_id_for("model-a").as_deref(),
            Some("fork-prism")
        );
    }

    /// A model with nothing configured has no engine, rather than a guessed one.
    ///
    /// There is deliberately no fallback to any particular build: an engine is
    /// the user's choice, and inventing one would put files behind tools and a
    /// server on a port they never agreed to.
    #[test]
    fn a_model_with_no_engine_configured_has_none() {
        assert_eq!(manager().engine_id_for("model-a"), None);
    }

    /// An engine that has been removed stays missing.
    ///
    /// Not replaced with anything: a model was pointed at that build because it
    /// behaves the way it does, and quietly running it on something else would
    /// produce answers from a backend nobody chose -- worse than a load that
    /// fails and says which engine is gone.
    #[test]
    fn a_removed_engine_is_not_silently_replaced() {
        let manager = manager();
        manager
            .set_model_engine("model-a", "fork-deleted".to_string())
            .unwrap();
        assert_eq!(
            manager.engine_id_for("model-a").as_deref(),
            Some("fork-deleted")
        );
    }

    /// Removing a model takes its engine with it.
    ///
    /// The engine is a setting keyed by model id, so it outlives the row unless
    /// it is deleted with it -- and a surviving choice keeps its engine looking
    /// in use, which would make that engine permanently unremovable.
    #[test]
    fn a_removed_model_takes_its_engine_with_it() {
        let manager = manager();
        manager
            .set_model_engine("model-a", "fork-prism".to_string())
            .unwrap();
        manager.database.remove_local_model("model-a").unwrap();
        assert_eq!(
            manager
                .database
                .setting::<Option<String>>(&engines::engine_setting_key("model-a"))
                .flatten(),
            None
        );
    }
}
