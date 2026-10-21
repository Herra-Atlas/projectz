//! The commands the download page calls.

use futures_util::StreamExt;
use tauri::AppHandle;

use super::catalog::{self, DownloadFile};
use super::client::{self, Client};
use super::install;
use super::kinds::Kind;

/// How many repositories a llama.cpp search expands into file lists.
///
/// A search returns thousands; each expansion is one request, so this is the
/// knob that decides how long a search takes. Thirty covers what a reader can
/// scroll through, and the ranking is by downloads -- so the repositories cut off
/// here are the ones nobody was going to download.
const REPOS: usize = 30;

/// How many of those requests are in flight at once. Small enough not to be
/// throttled, large enough that the wait is one round trip rather than thirty.
const PARALLEL: usize = 8;

/// The most rows the page can be handed in one answer.
const MAX_ROWS: usize = 300;

/// Files matching `query`, for one kind.
///
/// One command rather than one per kind: the page draws the same rows either way,
/// and the difference -- whether a search means "find repositories, then their
/// trees" or "read one index" -- belongs on this side of the wire.
#[tauri::command]
pub async fn downloads_search(kind: Kind, query: String) -> Result<Vec<DownloadFile>, String> {
    let client = Client::new()?;
    let mut files = match kind {
        Kind::Llama => {
            let repos = client.search_gguf(&query, REPOS).await?;
            // Cloned per task: a future that borrows the client cannot be polled
            // concurrently, because nothing in it owns what it points at.
            let http = client.http().clone();
            let trees = futures_util::stream::iter(repos.into_iter().map(|repo| {
                let http = http.clone();
                async move {
                    match client::tree(&http, &repo).await {
                        Ok(tree) => catalog::gguf_files(&repo, &tree),
                        // One unreadable repository does not fail the page; the
                        // others are still answers.
                        Err(_) => Vec::new(),
                    }
                }
            }))
            .buffer_unordered(PARALLEL)
            .collect::<Vec<_>>()
            .await;
            trees.into_iter().flatten().collect()
        }
        Kind::Tts => {
            let index = client.piper_index().await?;
            catalog::piper_voices(&index, &query)
        }
    };
    files.truncate(MAX_ROWS);
    Ok(files)
}

/// A file that is already on disk.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledFile {
    pub name: String,
    /// The repository folder it was installed under.
    pub repo: String,
    pub path: String,
    pub size: u64,
}

/// What has been downloaded for `kind` already.
///
/// Read from the folder rather than remembered in the database: the folder is the
/// truth -- a user who deletes a file has uninstalled it -- and a second record of
/// the same fact is one that can disagree.
#[tauri::command]
pub async fn downloads_installed(kind: Kind) -> Result<Vec<InstalledFile>, String> {
    let root = super::kinds::models_root_for(kind);
    let mut found = Vec::new();
    collect(&root, &mut found);
    found.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(found)
}

fn collect(directory: &std::path::Path, found: &mut Vec<InstalledFile>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, found);
            continue;
        }
        let usable = matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("gguf") | Some("onnx")
        );
        if !usable {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        let repo = path
            .parent()
            .and_then(|parent| parent.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        found.push(InstalledFile {
            name,
            repo,
            path: path.display().to_string(),
            size: std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0),
        });
    }
}

/// Downloads one row and returns where its file was written.
///
/// The path is reported rather than assumed by the caller: the installer is the
/// only side that knows the layout, and a second copy of that rule in the page is
/// how the two drift apart.
#[tauri::command]
pub async fn downloads_install(
    app: AppHandle,
    kind: Kind,
    repo: String,
    path: String,
) -> Result<String, String> {
    let client = Client::new()?;
    let written = install::download(&app, client.files(), kind, &repo, &path).await?;
    Ok(written
        .first()
        .map(|destination| destination.display().to_string())
        .unwrap_or_default())
}
