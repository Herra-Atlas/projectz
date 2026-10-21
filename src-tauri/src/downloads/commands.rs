//! The commands the download page calls.

use std::sync::Arc;

use futures_util::StreamExt;
use tauri::{AppHandle, Emitter, State};

use super::catalog::{self, DownloadFile};
use super::client::{self, Client};
use super::install::{self, Ending};
use super::jobs::{Flags, JobState, JobView, Jobs};
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
        // Two repositories, one engine: Whisper's own models and the Parakeet TDT
        // port that ships inside whisper.cpp. Each is one tree request, run
        // together.
        Kind::Stt => {
            let http = client.http().clone();
            // Owned, not `&'static str`: an async block that captures a borrowed
            // string cannot satisfy the `Send` bound the command macro asks for.
            let repos = [
                catalog::WHISPER_REPO.to_string(),
                catalog::PARAKEET_REPO.to_string(),
            ];
            let trees = futures_util::stream::iter(repos.into_iter().map(|repo| {
                let http = http.clone();
                let query = query.clone();
                async move {
                    match client::tree(&http, &repo).await {
                        Ok(tree) => catalog::stt_models(&repo, &tree, &query),
                        Err(_) => Vec::new(),
                    }
                }
            }))
            .buffer_unordered(2)
            .collect::<Vec<_>>()
            .await;
            let mut rows: Vec<DownloadFile> = trees.into_iter().flatten().collect();
            // Whisper first: it is the smaller, English-capable default, and a
            // mixed list sorted by name would open on Parakeet.
            rows.sort_by(|left, right| {
                let left_rank = if left.repo == catalog::WHISPER_REPO { 0 } else { 1 };
                let right_rank = if right.repo == catalog::WHISPER_REPO { 0 } else { 1 };
                left_rank.cmp(&right_rank).then_with(|| left.name.cmp(&right.name))
            });
            rows
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
    collect(&root, kind, &mut found);
    found.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(found)
}

fn collect(directory: &std::path::Path, kind: Kind, found: &mut Vec<InstalledFile>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, kind, found);
            continue;
        }
        // The extension of the file *this kind installs*, so a recognition model
        // is not filtered out for having the extension its own models use.
        let usable = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case(kind.extension()));
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

/// Starts a download and answers with its id.
///
/// Returns immediately: a model is minutes long, and a command that awaited it
/// would leave the page with no way to show progress, pause it, or leave.
#[tauri::command]
pub async fn downloads_start(
    app: AppHandle,
    jobs: State<'_, Arc<Jobs>>,
    kind: Kind,
    repo: String,
    path: String,
) -> Result<String, String> {
    let client = Client::new()?;
    let name = path.rsplit('/').next().unwrap_or(&path).to_string();
    let destination = kind.destination(&repo, &path);
    let (id, flags) = jobs.add(kind, &repo, &path, &name, &destination.display().to_string());
    let _ = app.emit("downloads-state", jobs.list());
    spawn(app, Arc::clone(&jobs), client.files().clone(), kind, repo, path, id.clone(), flags);
    Ok(id)
}

#[tauri::command]
pub fn downloads_jobs(jobs: State<'_, Arc<Jobs>>) -> Vec<JobView> {
    jobs.list()
}

/// Asks a running download to stop at the next chunk and keep its `.part`.
#[tauri::command]
pub fn downloads_pause(app: AppHandle, jobs: State<'_, Arc<Jobs>>, id: String) -> bool {
    let accepted = jobs.pause(&id);
    if accepted {
        let _ = app.emit("downloads-state", jobs.list());
    }
    accepted
}

/// Continues a paused download from the bytes already on disk.
#[tauri::command]
pub fn downloads_resume(app: AppHandle, jobs: State<'_, Arc<Jobs>>, id: String) -> Result<bool, String> {
    let Some(job) = jobs.resume(&id) else {
        return Ok(false);
    };
    let client = Client::new()?;
    let _ = app.emit("downloads-state", jobs.list());
    spawn(
        app,
        Arc::clone(&jobs),
        client.files().clone(),
        job.kind,
        job.repo,
        job.path,
        job.id,
        Arc::clone(&jobs.flags(&id).ok_or("The download is no longer in the list.")?),
    );
    Ok(true)
}

/// Stops a download and discards what it had written.
#[tauri::command]
pub fn downloads_stop(app: AppHandle, jobs: State<'_, Arc<Jobs>>, id: String) -> bool {
    let accepted = jobs.stop(&id);
    if accepted {
        let _ = app.emit("downloads-state", jobs.list());
    }
    accepted
}

/// Runs one job to its end, reporting into the list as it goes.
///
/// The companions of a file are fetched here too -- a Piper voice's `.onnx.json`
/// is part of the voice -- which is why this is a loop rather than one call.
fn spawn(
    app: AppHandle,
    jobs: Arc<Jobs>,
    client: reqwest::Client,
    kind: Kind,
    repo: String,
    path: String,
    id: String,
    flags: Arc<Flags>,
) {
    tauri::async_runtime::spawn(async move {
        let mut ending: Result<Ending, String> = Ok(Ending::Done);
        for companion in kind.companions(&path) {
            let destination = kind.destination(&repo, &companion);
            let primary = companion == path;
            // Throttled: a chunk arrives every few milliseconds and the whole list
            // per chunk would be more traffic than the download.
            let mut last = std::time::Instant::now();
            let outcome = install::fetch(
                &client,
                &repo,
                &companion,
                &destination,
                &flags,
                |received, total| {
                    if !primary {
                        return;
                    }
                    jobs.progress(&id, received, total);
                    if last.elapsed().as_millis() >= 200 {
                        last = std::time::Instant::now();
                        let _ = app.emit("downloads-state", jobs.list());
                    }
                },
            )
            .await;

            match outcome {
                Ok(Ending::Done) => continue,
                Ok(other) => {
                    ending = Ok(other);
                    break;
                }
                Err(error) => {
                    ending = Err(error);
                    break;
                }
            }
        }

        match ending {
            Ok(Ending::Done) => {
                let destination = kind.destination(&repo, &path);
                let size = std::fs::metadata(&destination)
                    .map(|meta| meta.len())
                    .unwrap_or(0);
                jobs.progress(&id, size, Some(size));
                jobs.state(&id, JobState::Done, None);
                // A downloaded GGUF is not a model this app offers until it is
                // registered, and one the user has to find again by hand is a
                // download that looks like it did nothing.
                if kind == Kind::Llama {
                    use tauri::Manager;
                    let runtime = app.state::<crate::ai::runtime::AiRuntime>();
                    let _ = runtime.local_models().add(destination.display().to_string());
                }
                let _ = app.emit("downloads-installed", kind);
            }
            Ok(Ending::Paused) => jobs.state(&id, JobState::Paused, None),
            Ok(Ending::Stopped) => jobs.state(&id, JobState::Stopped, None),
            Err(error) => jobs.state(&id, JobState::Failed, Some(error)),
        }
        let _ = app.emit("downloads-state", jobs.list());
    });
}
