use std::path::Path;
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use super::{engines, LocalModelManager};

/// Where a batch import has got to, emitted once before and once after each file.
///
/// `index` is 1-based and `done` says whether that file has been handled yet, so
/// the frontend can show a fraction without counting events itself: a `done`
/// event moves the bar, an open one names the file being read.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddProgress {
    pub index: usize,
    pub total: usize,
    pub name: String,
    pub done: bool,
    /// Why this file could not be added, when it could not. Absent on success.
    pub error: Option<String>,
}

/// Registers every dropped GGUF file, reporting each one as it is read.
///
/// **The loop runs on a blocking thread.** Reading a GGUF header is file I/O --
/// up to 64 MB per model on a cold cache -- and a synchronous command would run
/// it on the main thread, freezing the window for as long as the drop took. That
/// is exactly what made a multi-file drop look like it had been ignored. Here the
/// UI keeps painting and the notification bar follows the events instead.
///
/// One file failing does not stop the rest: a duplicate or an unreadable file is
/// reported on its own row and the remaining ones still land.
pub async fn add_many(
    app: AppHandle,
    manager: Arc<LocalModelManager>,
    paths: Vec<String>,
    engine_id: Option<String>,
) -> Result<usize, String> {
    if paths.is_empty() {
        return Ok(0);
    }
    // Checked once, before any file is written: an engine that is not installed
    // would otherwise fail on the last step of every file in the batch.
    if let Some(engine) = engine_id.as_deref() {
        engines::require_installed(&app, engine)?;
    }

    tauri::async_runtime::spawn_blocking(move || {
        let total = paths.len();
        let mut added = 0usize;
        for (position, path) in paths.into_iter().enumerate() {
            let name = Path::new(&path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("model")
                .to_string();
            let _ = app.emit(
                "local-model-add-progress",
                AddProgress {
                    index: position + 1,
                    total,
                    name: name.clone(),
                    done: false,
                    error: None,
                },
            );

            let outcome = manager.add(path).and_then(|model| {
                if let Some(engine) = engine_id.as_deref() {
                    manager.set_model_engine(&model.id, engine.to_string())?;
                }
                Ok(())
            });
            let error = match outcome {
                Ok(()) => {
                    added += 1;
                    None
                }
                Err(error) => Some(error),
            };

            let _ = app.emit(
                "local-model-add-progress",
                AddProgress {
                    index: position + 1,
                    total,
                    name,
                    done: true,
                    error,
                },
            );
        }
        added
    })
    .await
    .map_err(|error| format!("Adding models did not finish: {error}"))
}
