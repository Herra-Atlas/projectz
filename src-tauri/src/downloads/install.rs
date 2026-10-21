//! Fetching one file into its place.
//!
//! # Why the bytes go to a `.part` file first
//!
//! A half-written GGUF is indistinguishable from a whole one until something
//! tries to load it, and a `.part` name is what makes an interrupted download
//! visibly not a model. The rename at the end is atomic on the same volume, so a
//! file that appears under its real name is a file that finished.
//!
//! # Why the body is streamed
//!
//! A model is gigabytes. Buffering it would cost the memory of the model before
//! it was ever used, for a copy that is thrown away; the chunks go straight to
//! the file, and the same loop is what reports progress.

use std::io::Write;
use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use tauri::{AppHandle, Emitter};

use super::kinds::Kind;

const API: &str = "https://huggingface.co";

/// What a row is told while its file is arriving.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub repo: String,
    pub path: String,
    pub received: u64,
    /// Absent when the server did not send a length, which is not an error.
    pub total: Option<u64>,
}

/// Downloads everything `path` needs, and returns what was written.
///
/// The companions are fetched too -- a Piper voice's `.onnx.json` is part of the
/// voice, not a later extra -- so a row that says "installed" is a row that can
/// actually be loaded.
pub async fn download(
    app: &AppHandle,
    client: &reqwest::Client,
    kind: Kind,
    repo: &str,
    path: &str,
) -> Result<Vec<PathBuf>, String> {
    let mut written = Vec::new();
    for companion in kind.companions(path) {
        let destination = kind.destination(repo, &companion);
        // Only the file that was asked for reports progress. The sidecar is a few
        // kilobytes, and a second bar that fills instantly is noise.
        let report = if companion == path {
            Some(Progress {
                repo: repo.to_string(),
                path: path.to_string(),
                received: 0,
                total: None,
            })
        } else {
            None
        };
        fetch(app, client, repo, &companion, &destination, report).await?;
        written.push(destination);
    }
    Ok(written)
}

async fn fetch(
    app: &AppHandle,
    client: &reqwest::Client,
    repo: &str,
    path: &str,
    destination: &Path,
    progress: Option<Progress>,
) -> Result<(), String> {
    let url = format!("{API}/{repo}/resolve/main/{path}");
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|error| format!("Could not download {path}: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Could not download {path}: HuggingFace answered {}",
            response.status().as_u16()
        ));
    }

    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
    }

    let mut partial = destination.as_os_str().to_os_string();
    partial.push(".part");
    let partial = PathBuf::from(partial);

    let mut file = std::fs::File::create(&partial)
        .map_err(|error| format!("Could not write {}: {error}", partial.display()))?;

    let total = response.content_length();
    let mut received: u64 = 0;
    let mut body = response.bytes_stream();
    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(|error| {
            // The partial file is left behind on purpose: it is the evidence that
            // this download did not finish, and the next attempt replaces it.
            format!("The download of {path} was interrupted: {error}")
        })?;
        file.write_all(&chunk)
            .map_err(|error| format!("Could not write {}: {error}", partial.display()))?;
        received += chunk.len() as u64;

        if let Some(template) = &progress {
            let _ = app.emit(
                "downloads-progress",
                Progress {
                    received,
                    total,
                    ..template.clone()
                },
            );
        }
    }

    // Flushed before the rename so the size a reader stats afterwards is the size
    // that was written.
    file.flush()
        .map_err(|error| format!("Could not finish {}: {error}", partial.display()))?;
    drop(file);

    std::fs::rename(&partial, destination).map_err(|error| {
        format!(
            "Could not put {} in place: {error}",
            destination.display()
        )
    })?;
    Ok(())
}
