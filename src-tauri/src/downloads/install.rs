//! Fetching one file into its place.
//!
//! # Why the bytes go to a `.part` file first
//!
//! A half-written GGUF is indistinguishable from a whole one until something
//! tries to load it, and a `.part` name is what makes an interrupted download
//! visibly not a model. The rename at the end is atomic on the same volume, so a
//! file that appears under its real name is a file that finished.
//!
//! # Why the same file is the resume point
//!
//! Pausing leaves the `.part` where it is, and the next attempt asks for
//! `bytes=<its length>-`. If the server ignores the range and answers 200, the
//! file is truncated and the transfer starts again -- appending would put two
//! copies of the beginning in the middle of a model.
//!
//! # Why the body is streamed
//!
//! A model is gigabytes. Buffering it would cost the memory of the model before
//! it was ever used, for a copy that is thrown away; the chunks go straight to
//! the file, and the same loop is what reports progress.

use std::io::Write;
use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use reqwest::header::RANGE;
use reqwest::StatusCode;

use super::jobs::Flags;

const API: &str = "https://huggingface.co";

/// How a transfer ended. Only `Done` leaves a file under its real name.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Ending {
    Done,
    Paused,
    Stopped,
}

/// Where a file is written while it is incomplete.
pub fn partial_of(destination: &Path) -> PathBuf {
    let mut name = destination.as_os_str().to_os_string();
    name.push(".part");
    PathBuf::from(name)
}

/// Streams one repository file to `destination`.
pub async fn fetch(
    client: &reqwest::Client,
    repo: &str,
    path: &str,
    destination: &Path,
    flags: &Flags,
    mut report: impl FnMut(u64, Option<u64>),
) -> Result<Ending, String> {
    // A Hub repository resolves through `resolve/main`; a GitHub one is served
    // raw. Paths there can contain spaces (`Hey Lara/lara.onnx`), and a URL with
    // a space in it is not a URL.
    let url = if repo == crate::downloads::catalog::WAKEWORDS_REPO {
        format!(
            "https://raw.githubusercontent.com/{repo}/main/{}",
            path.replace(' ', "%20")
        )
    } else {
        format!("{API}/{repo}/resolve/main/{path}")
    };
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
    }
    let partial = partial_of(destination);
    let already = std::fs::metadata(&partial)
        .map(|meta| meta.len())
        .unwrap_or(0);

    let mut request = client.get(&url);
    if already > 0 {
        request = request.header(RANGE, format!("bytes={already}-"));
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("Could not download {path}: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Could not download {path}: HuggingFace answered {}",
            response.status().as_u16()
        ));
    }

    let resumed = response.status() == StatusCode::PARTIAL_CONTENT;
    let mut file = if resumed {
        std::fs::OpenOptions::new()
            .append(true)
            .open(&partial)
            .map_err(|error| format!("Could not write {}: {error}", partial.display()))?
    } else {
        std::fs::File::create(&partial)
            .map_err(|error| format!("Could not write {}: {error}", partial.display()))?
    };

    let mut received = if resumed { already } else { 0 };
    let total = response
        .content_length()
        .map(|length| length + if resumed { already } else { 0 });
    report(received, total);

    let mut body = response.bytes_stream();
    while let Some(chunk) = body.next().await {
        if flags.stopped() {
            drop(file);
            let _ = std::fs::remove_file(&partial);
            return Ok(Ending::Stopped);
        }
        if flags.paused() {
            let _ = file.flush();
            return Ok(Ending::Paused);
        }
        let chunk = chunk.map_err(|error| {
            // The partial file is left behind on purpose: it is both the evidence
            // that this download did not finish and the resume point.
            format!("The download of {path} was interrupted: {error}")
        })?;
        file.write_all(&chunk)
            .map_err(|error| format!("Could not write {}: {error}", partial.display()))?;
        received += chunk.len() as u64;
        report(received, total);
    }

    // Flushed before the rename so the size a reader stats afterwards is the size
    // that was written.
    file.flush()
        .map_err(|error| format!("Could not finish {}: {error}", partial.display()))?;
    drop(file);

    if flags.stopped() {
        let _ = std::fs::remove_file(&partial);
        return Ok(Ending::Stopped);
    }

    std::fs::rename(&partial, destination)
        .map_err(|error| format!("Could not put {} in place: {error}", destination.display()))?;
    Ok(Ending::Done)
}
