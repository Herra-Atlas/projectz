use std::{
    fs,
    net::{IpAddr, SocketAddr},
    path::Path,
};

use base64::Engine;
use futures_util::StreamExt;
use tauri::{AppHandle, Emitter, Manager, State};
use tracing::info;

use crate::ai::runtime::AiRuntime;
use crate::ai::types::ChatRequest;
use crate::database::jobs::{Job, JobRun, NewJob};

#[tauri::command]
pub async fn ai_chat(
    app: AppHandle,
    runtime: State<'_, AiRuntime>,
    request: ChatRequest,
) -> Result<crate::ai::types::ChatResult, String> {
    info!(run_id = %request.run_id, endpoint = ?request.endpoint_id, "ai_chat start");
    Ok(runtime.run_chat(&app, request).await)
}

#[tauri::command]
pub fn ai_cancel_chat(runtime: State<'_, AiRuntime>, run_id: String) {
    runtime.cancel_chat(&run_id);
}

#[tauri::command]
pub async fn ai_skip_local_reasoning(
    runtime: State<'_, AiRuntime>,
    run_id: String,
) -> Result<(), String> {
    runtime.skip_local_reasoning(&run_id).await
}

/// Answers a pending tool prompt.
///
/// Returns whether an outstanding prompt was actually waiting. `false` means the
/// user answered a dialog for a call that had already been stopped or already
/// answered -- a stale dialog, which is a normal outcome and not an error the
/// caller should retry on.
#[tauri::command]
pub fn ai_answer_approval(runtime: State<'_, AiRuntime>, approval_id: String, allow: bool) -> bool {
    runtime.answer_approval(&approval_id, allow)
}

/// Tells the backend which session is on screen so a reply finishing in it stays silent.
#[tauri::command]
pub fn ai_set_viewed_session(runtime: State<'_, AiRuntime>, session_id: Option<String>) {
    runtime.set_viewed_session(session_id);
}

/// Generate a short conversation title from the user's opening message,
/// trying each configured model in order. The frontend passes the persisted
/// list so the backend stays the single source of truth for provider keys and
/// routing.
#[tauri::command]
pub async fn ai_generate_title(
    runtime: State<'_, AiRuntime>,
    selections: Vec<serde_json::Value>,
    prompt: String,
) -> Result<String, String> {
    info!(count = selections.len(), "ai_generate_title start");
    runtime.generate_title(&selections, &prompt).await
}

#[tauri::command]
pub async fn database_initialize(
    app: AppHandle,
    runtime: State<'_, AiRuntime>,
) -> Result<(), String> {
    let _ = app.emit(
        "startup-progress",
        serde_json::json!({"phase":"database","message":"Checking database","progress":12}),
    );
    runtime.database().initialize()?;
    let legacy_data_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("projectz");
    runtime.database().import_legacy(&legacy_data_dir)?;
    // The skills the app ships, written once (the seed is idempotent and is
    // short-circuited by a setting). A failure is logged rather than fatal: a
    // database that cannot take these rows is not a reason to refuse to start.
    if let Err(error) = runtime.database().seed_bundled_skills() {
        tracing::warn!(%error, "bundled skills could not be seeded");
    }
    let _ = app.emit(
        "startup-progress",
        serde_json::json!({"phase":"migrations","message":"Database ready","progress":38}),
    );
    let endpoints = runtime.endpoints().list();
    if endpoints.is_empty() {
        let _ = app.emit(
            "startup-progress",
            serde_json::json!({"phase":"models","message":"No providers to refresh","progress":96}),
        );
        return Ok(());
    }
    for (index, mut endpoint) in endpoints.iter().cloned().enumerate() {
        let _ = app.emit("startup-progress", serde_json::json!({"phase":"models","message":format!("Refreshing models: {}", endpoint.name),"progress":38 + (((index + 1) as f32 / endpoints.len() as f32) * 58.0) as u8}));
        let refreshed = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            fetch_models_with_capabilities(&endpoint),
        )
        .await
        .unwrap_or_else(|_| Err("Provider model refresh timed out".to_string()));
        if let Ok(models) = refreshed {
            let model_ids: Vec<String> = models.iter().map(|(model, _)| model.clone()).collect();
            // The same reconciliation a manual test performs: a launch refresh is
            // the other moment the provider's catalogue is known to be current, so
            // a model it has stopped listing is dropped and a new one arrives off.
            let _ = runtime
                .database()
                .reconcile_provider_models(&endpoint.id, &model_ids);
            endpoint.models = model_ids;
            runtime.endpoints().update(endpoint)?;
        }
    }
    let _ = app.emit(
        "startup-progress",
        serde_json::json!({"phase":"ready","message":"Startup complete","progress":100}),
    );
    Ok(())
}

#[tauri::command]
pub fn database_frontend_imported(runtime: State<'_, AiRuntime>) -> bool {
    runtime.database().frontend_imported()
}

#[tauri::command]
pub fn database_import_frontend(
    runtime: State<'_, AiRuntime>,
    sessions: Vec<serde_json::Value>,
    general_settings: serde_json::Value,
) -> Result<(), String> {
    let database = runtime.database();
    if database.frontend_imported() {
        return Ok(());
    }
    database.import_frontend_data(&sessions, &general_settings)
}

#[tauri::command]
pub fn database_list_sessions(
    runtime: State<'_, AiRuntime>,
) -> Result<Vec<serde_json::Value>, String> {
    runtime.database().list_chat_sessions()
}

/// Every conversation, without any messages.
///
/// The launch read. The sidebar draws titles, ordering, and a dot, and the token
/// rollup lives on the `sessions` row, so none of that needs a transcript -- and
/// fetching every message in the database to draw a list of titles is what made
/// startup grow with history instead of with the number of conversations.
#[tauri::command]
pub fn database_list_session_headers(
    runtime: State<'_, AiRuntime>,
) -> Result<Vec<serde_json::Value>, String> {
    runtime.database().list_session_headers()
}

/// The sub-agent runs, newest first, optionally for one parent conversation.
///
/// Separate from `database_list_session_headers` rather than a flag on it,
/// because the two never want the same thing: the sidebar lists conversations and
/// must not show a run, and the Sub agents panel lists runs and must not show a
/// conversation. One query with a switch would be a query that can be called the
/// wrong way.
#[tauri::command]
pub fn database_list_subagents(
    runtime: State<'_, AiRuntime>,
    parent_session_id: Option<String>,
) -> Result<Vec<serde_json::Value>, String> {
    runtime
        .database()
        .list_subagent_headers(parent_session_id.as_deref())
}

/// The sub-agents running right now.
///
/// A run is only written to the database when it finishes, so this is the one way
/// a panel opened mid-run can show an agent already at work. The list is
/// in-memory and per-process, which is exactly the lifetime it describes: it is
/// empty at launch and clears itself as each run ends.
#[tauri::command]
pub fn ai_running_subagents(
    runtime: State<'_, AiRuntime>,
) -> Vec<crate::ai::runtime::RunningSubAgent> {
    runtime.running_subagents()
}

/// One conversation's messages, oldest first.
///
/// Called when a conversation is opened, so a session whose transcript has not
/// been fetched yet has no `messages` in the frontend either. Callers must treat
/// that as "not loaded", never as "empty" -- see `save_chat_session_tx` for why a
/// short array is refused rather than written.
#[tauri::command]
pub fn database_list_session_messages(
    runtime: State<'_, AiRuntime>,
    session_id: String,
) -> Result<Vec<serde_json::Value>, String> {
    runtime.database().list_session_messages(&session_id)
}

#[tauri::command]
pub fn database_save_session(
    runtime: State<'_, AiRuntime>,
    session: serde_json::Value,
) -> Result<(), String> {
    runtime.database().save_chat_session(&session)
}

/// Deletes a stored conversation.
///
/// A transcript a job produced takes that job with it, along with the job's other
/// transcripts. To the reader the two are one thing -- they delete the run they can
/// see and expect the job that made it to go too -- and the alternative leaves a job
/// in the list pointing at a conversation that no longer exists.
#[tauri::command]
pub fn database_delete_session(
    app: AppHandle,
    runtime: State<'_, AiRuntime>,
    id: String,
) -> Result<(), String> {
    runtime.database().delete_chat_session(&id)?;
    match runtime.database().delete_job_for_session(&id) {
        Ok(Some(job_id)) => {
            let _ = app.emit(
                "job-event",
                serde_json::json!({ "jobId": job_id, "status": "deleted" }),
            );
        }
        Ok(None) => {}
        // The conversation is gone either way, and failing the delete over a
        // follow-up would report a failure that did not happen.
        Err(error) => tracing::warn!(%error, "the job behind a deleted transcript was not removed"),
    }
    Ok(())
}

#[tauri::command]
pub fn database_clear_sessions(runtime: State<'_, AiRuntime>) -> Result<usize, String> {
    runtime.database().clear_chat_sessions()
}

#[tauri::command]
pub fn database_get_setting(
    runtime: State<'_, AiRuntime>,
    key: String,
) -> Option<serde_json::Value> {
    runtime.database().setting(&key)
}

#[tauri::command]
pub fn database_set_setting(
    runtime: State<'_, AiRuntime>,
    key: String,
    value: serde_json::Value,
) -> Result<(), String> {
    runtime.database().set_setting(&key, &value)
}

#[tauri::command]
pub fn database_tables(runtime: State<'_, AiRuntime>) -> Result<Vec<String>, String> {
    runtime.database().table_names()
}

/// The open workspaces and the current selection.
#[tauri::command]
pub fn workspaces_list(runtime: State<'_, AiRuntime>) -> crate::database::workspaces::Workspaces {
    runtime.workspaces()
}

/// Adds a folder to the top of the list and points the tools at it.
#[tauri::command]
pub fn workspace_open(
    runtime: State<'_, AiRuntime>,
    path: String,
) -> Result<crate::database::workspaces::Workspaces, String> {
    runtime.open_workspace(&path)
}

/// Points the tools at a folder that is already open.
#[tauri::command]
pub fn workspace_select(
    runtime: State<'_, AiRuntime>,
    path: String,
) -> Result<crate::database::workspaces::Workspaces, String> {
    runtime.select_workspace(&path)
}

/// Drops a folder, moving the root if it was the chosen one.
#[tauri::command]
pub fn workspace_close(
    runtime: State<'_, AiRuntime>,
    path: String,
) -> Result<crate::database::workspaces::Workspaces, String> {
    runtime.close_workspace(&path)
}

/// Every skill, enabled or not. For the management screen.
#[tauri::command]
pub fn skills_list(
    runtime: State<'_, AiRuntime>,
) -> Result<Vec<crate::database::skills::Skill>, String> {
    runtime.database().list_skills()
}

/// Only the enabled skills, which is what a reply may use.
#[tauri::command]
pub fn skills_enabled(
    runtime: State<'_, AiRuntime>,
) -> Result<Vec<crate::database::skills::Skill>, String> {
    runtime.database().enabled_skills()
}

/// Creates a skill. The id and timestamps are generated here rather than accepted
/// from the frontend, so a caller cannot backdate or overwrite one.
#[tauri::command]
pub fn skills_create(
    runtime: State<'_, AiRuntime>,
    skill: crate::database::skills::NewSkill,
) -> Result<crate::database::skills::Skill, String> {
    runtime.database().create_skill(&skill)
}

/// Replaces a skill's text, keeping its id, origin and usage count.
#[tauri::command]
pub fn skills_update(
    runtime: State<'_, AiRuntime>,
    id: String,
    skill: crate::database::skills::NewSkill,
) -> Result<crate::database::skills::Skill, String> {
    runtime.database().update_skill(&id, &skill)
}

/// Turns a skill on or off. Disabling is preferred to deleting: the work of
/// writing a skill is not undone by a switch.
#[tauri::command]
pub fn skills_set_enabled(
    runtime: State<'_, AiRuntime>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    runtime.database().set_skill_enabled(&id, enabled)
}

#[tauri::command]
pub fn skills_delete(runtime: State<'_, AiRuntime>, id: String) -> Result<(), String> {
    runtime.database().delete_skill(&id)
}

/// Every saved job, for the editor's list.
#[tauri::command]
pub fn job_list(runtime: State<'_, AiRuntime>) -> Result<Vec<Job>, String> {
    runtime.database().list_jobs()
}

/// Creates a job from the editor.
#[tauri::command]
pub fn job_create(runtime: State<'_, AiRuntime>, job: NewJob) -> Result<Job, String> {
    let next = crate::jobs::first_run(&job.schedule, chrono::Utc::now());
    runtime.database().create_job(&job, next.as_deref())
}

/// Replaces a job's editable fields, and re-arms its clock.
///
/// The schedule is re-read here rather than left alone, because editing a job is
/// how its time changes: a job moved from "every 5 minutes" to "once at 3am" that
/// kept its old next-run would fire at the wrong moment exactly once.
#[tauri::command]
pub fn job_update(runtime: State<'_, AiRuntime>, id: String, job: NewJob) -> Result<Job, String> {
    runtime.database().update_job(&id, &job)?;
    let next = crate::jobs::first_run(&job.schedule, chrono::Utc::now());
    runtime.database().set_job_next_run(&id, next.as_deref())?;
    runtime
        .database()
        .job(&id)?
        .ok_or_else(|| format!("No job with id {id}"))
}

#[tauri::command]
pub fn job_delete(runtime: State<'_, AiRuntime>, id: String) -> Result<(), String> {
    runtime.database().delete_job(&id)
}

/// Turns a job on or off. Enabling re-arms its clock from the schedule, so a
/// one-shot switched back on waits for its moment rather than running at once.
#[tauri::command]
pub fn job_set_enabled(
    runtime: State<'_, AiRuntime>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    runtime.database().set_job_enabled(&id, enabled)?;
    if enabled {
        if let Some(job) = runtime.database().job(&id)? {
            let next = crate::jobs::first_run(&job.schedule, chrono::Utc::now());
            runtime.database().set_job_next_run(&id, next.as_deref())?;
        }
    }
    Ok(())
}

/// Total RAM and VRAM, so the editor can show a limit as the bytes it means.
///
/// Reported rather than computed in the frontend because only the backend can read
/// either figure, and a second implementation of "how much memory is there" would
/// be a second answer.
#[derive(serde::Serialize)]
pub struct MachineTotals {
    pub ram_total_bytes: Option<u64>,
    /// `None` when the build has no GPU query, which is the default. A limit on a
    /// figure we cannot read is simply not enforced.
    pub vram_total_bytes: Option<u64>,
}

#[tauri::command]
pub fn job_machine_totals() -> MachineTotals {
    MachineTotals {
        ram_total_bytes: crate::jobs::ram_total_bytes(),
        vram_total_bytes: crate::ai::local::device::DeviceMemory::local()
            .map(|device| device.total_bytes),
    }
}

/// One job's recent attempts, newest first.
#[tauri::command]
pub fn job_runs(runtime: State<'_, AiRuntime>, id: String) -> Result<Vec<JobRun>, String> {
    runtime.database().job_runs(&id, 50)
}

/// Starts a job immediately, bypassing its clock and conditions but not its lane.
#[tauri::command]
pub fn job_run_now(
    app: AppHandle,
    runtime: State<'_, AiRuntime>,
    scheduler: State<'_, crate::jobs::Scheduler>,
    id: String,
) -> Result<(), String> {
    scheduler.run_now(&app, &runtime, &id)
}

/// Whether the scheduler is paused, and which jobs are running.
#[tauri::command]
pub fn job_scheduler_status(
    scheduler: State<'_, crate::jobs::Scheduler>,
) -> serde_json::Value {
    serde_json::json!({
        "paused": scheduler.is_paused(),
        // Ids rather than names: the window already has the jobs and can name
        // them, and looking them up here would be a second read of the same rows.
        "running": scheduler.running_jobs(),
    })
}

/// The pause switch: one control that stops every job from starting.
#[tauri::command]
pub fn job_scheduler_pause(
    app: AppHandle,
    scheduler: State<'_, crate::jobs::Scheduler>,
    paused: bool,
) {
    scheduler.set_paused(paused);
    // The tray shows the same switch, so it is told rather than left to disagree.
    crate::sync_jobs_toggle(&app, paused);
}

#[tauri::command]
pub fn database_table_rows(
    runtime: State<'_, AiRuntime>,
    table: String,
    offset: u32,
    limit: u32,
) -> Result<serde_json::Value, String> {
    runtime.database().table_rows(&table, offset, limit)
}

/// Usage aggregates for the statistics page.
///
/// `range`, `since` and `bucket_sql` come from the frontend's fixed timeframe
/// table. `bucket_sql` is interpolated into the timeline query, so it is only
/// ever a value from that table; an unrecognised key is rejected rather than
/// passed through.
#[tauri::command]
pub fn database_usage_report(
    runtime: State<'_, AiRuntime>,
    range: String,
    since: String,
    bucket_sql: String,
) -> Result<crate::database::statistics::UsageReport, String> {
    if !TIMEFRAME_BUCKETS.contains(&bucket_sql.as_str()) {
        return Err("Unknown statistics bucket".into());
    }
    runtime.database().usage_report(&range, &since, &bucket_sql)
}

/// The only bucket expressions the statistics page may request.
///
/// Must stay in step with `TIMEFRAMES` in
/// `src/features/statistics/timeframes.ts`, which supplies these literals.
const TIMEFRAME_BUCKETS: &[&str] = &[
    "substr(created_at,1,13)",
    "substr(created_at,1,10)",
    "substr(created_at,1,7)",
    "substr(created_at,1,4)",
];

#[tauri::command]
pub fn extract_pdf_attachment(
    name: String,
    data_base64: String,
) -> Result<crate::ai::types::ChatAttachment, String> {
    const MAX_ATTACHMENT_SIZE: usize = 15 * 1024 * 1024;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data_base64)
        .map_err(|_| "Invalid PDF data".to_string())?;
    if bytes.len() > MAX_ATTACHMENT_SIZE {
        return Err("PDF files must be 15 MB or smaller".to_string());
    }
    let text = pdf_extract::extract_text_from_mem(&bytes)
        .map_err(|error| format!("Could not extract text from PDF: {error}"))?;
    if text.trim().is_empty() {
        return Err(
            "This PDF has no extractable text. Scanned PDFs are not supported yet.".to_string(),
        );
    }
    Ok(crate::ai::types::ChatAttachment {
        name,
        mime_type: "text/plain".to_string(),
        data_base64: base64::engine::general_purpose::STANDARD.encode(text.as_bytes()),
    })
}

#[tauri::command]
pub fn read_chat_attachment(path: String) -> Result<crate::ai::types::ChatAttachment, String> {
    const MAX_ATTACHMENT_SIZE: u64 = 15 * 1024 * 1024;
    let file_path = Path::new(&path);
    let metadata =
        fs::metadata(file_path).map_err(|error| format!("Could not read file: {error}"))?;
    if !metadata.is_file() {
        return Err("Only files can be attached".to_string());
    }
    if metadata.len() > MAX_ATTACHMENT_SIZE {
        return Err("Files must be 15 MB or smaller".to_string());
    }
    let name = file_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "The file name is not valid UTF-8".to_string())?;
    let extension = file_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mime_type = match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "pdf" => "application/pdf",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "txt" | "log" => "text/plain",
        "md" | "markdown" => "text/markdown",
        "json" => "application/json",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "xml" => "application/xml",
        "yaml" | "yml" => "text/yaml",
        "rs" => "text/x-rust",
        "py" => "text/x-python",
        "js" | "jsx" | "ts" | "tsx" | "css" | "toml" | "sql" | "sh" => "text/plain",
        _ => {
            return Err(
                "This file type is not supported yet. Attach an image or text/code file."
                    .to_string(),
            )
        }
    };
    let bytes = fs::read(file_path).map_err(|error| format!("Could not read file: {error}"))?;
    if mime_type == "application/pdf" {
        let text = pdf_extract::extract_text_from_mem(&bytes)
            .map_err(|error| format!("Could not extract text from PDF: {error}"))?;
        if text.trim().is_empty() {
            return Err(
                "This PDF has no extractable text. Scanned PDFs are not supported yet.".to_string(),
            );
        }
        return Ok(crate::ai::types::ChatAttachment {
            name: name.to_string(),
            mime_type: "text/plain".to_string(),
            data_base64: base64::engine::general_purpose::STANDARD.encode(text.as_bytes()),
        });
    }
    if !mime_type.starts_with("image/") && std::str::from_utf8(&bytes).is_err() {
        return Err("This text file is not valid UTF-8".to_string());
    }
    Ok(crate::ai::types::ChatAttachment {
        name: name.to_string(),
        mime_type: mime_type.to_string(),
        data_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
    })
}

fn provider_icon_domain(base_url: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(base_url).map_err(|_| "Invalid provider URL".to_string())?;
    if url.scheme() != "https" {
        return Err("Provider icons require an HTTPS provider URL".to_string());
    }
    let raw_host = url
        .host_str()
        .ok_or_else(|| "Provider URL has no hostname".to_string())?;
    let host = raw_host.strip_prefix("api.").unwrap_or(raw_host);
    if host.parse::<IpAddr>().is_ok() || host.is_empty() {
        return Err("Provider icons require a public hostname".to_string());
    }
    Ok(host.to_ascii_lowercase())
}

fn is_public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => {
            let [a, b, _, _] = ip.octets();
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_broadcast()
                && !ip.is_multicast()
                && !ip.is_unspecified()
                && a != 0
                && !(a == 100 && (64..=127).contains(&b))
                && !(a == 169 && b == 254)
                && !(a == 192 && b == 0)
                && !(a == 192 && b == 88 && ip.octets()[2] == 99)
                && !(a == 198 && (b == 18 || b == 19))
                && !(a == 198 && b == 51 && ip.octets()[2] == 100)
                && !(a == 203 && b == 0 && ip.octets()[2] == 113)
                && a < 240
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            (segments[0] & 0xe000) == 0x2000
                && !(segments[0] == 0x2001 && segments[1] == 0x0db8)
                && !ip.is_multicast()
                && !ip.is_loopback()
                && !ip.is_unspecified()
        }
    }
}

fn favicon_data_url(bytes: &[u8], mime: &str) -> String {
    format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

fn favicon_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 13, 10, 0x1a, 10]) {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else if bytes.starts_with(b"BM") {
        Some("image/bmp")
    } else if bytes.starts_with(&[0, 0, 1, 0]) {
        Some("image/x-icon")
    } else {
        None
    }
}

#[tauri::command]
pub async fn provider_favicon(app: AppHandle, base_url: String) -> Result<Option<String>, String> {
    const MAX_ICON_BYTES: usize = 256 * 1024;

    let host = provider_icon_domain(&base_url)?;
    let cache_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("favicons");
    fs::create_dir_all(&cache_dir).map_err(|error| error.to_string())?;
    let cache_path = cache_dir.join(format!("{host}.favicon"));

    if let Ok(metadata) = fs::metadata(&cache_path) {
        if metadata.len() as usize <= MAX_ICON_BYTES {
            if let Ok(bytes) = fs::read(&cache_path) {
                if let Some(mime) = favicon_mime(&bytes) {
                    return Ok(Some(favicon_data_url(&bytes, mime)));
                }
            }
        }
    }

    let addresses: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 443))
        .await
        .map_err(|_| "Provider hostname could not be resolved".to_string())?
        .collect();
    if addresses.is_empty() || addresses.iter().any(|address| !is_public_ip(address.ip())) {
        return Ok(None);
    }

    let pinned_addresses: Vec<SocketAddr> = addresses;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .resolve_to_addrs(&host, &pinned_addresses)
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .get(format!("https://{host}/favicon.ico"))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Ok(None);
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_ICON_BYTES as u64)
    {
        return Ok(None);
    }

    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| error.to_string())?;
        if bytes.len() + chunk.len() > MAX_ICON_BYTES {
            return Ok(None);
        }
        bytes.extend_from_slice(&chunk);
    }
    let Some(mime) = favicon_mime(&bytes) else {
        return Ok(None);
    };

    let temporary_path = cache_path.with_extension(format!(
        "{}.{}.tmp",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    fs::write(&temporary_path, &bytes).map_err(|error| error.to_string())?;
    if cache_path.exists() {
        fs::remove_file(&cache_path).map_err(|error| error.to_string())?;
    }
    fs::rename(&temporary_path, &cache_path).map_err(|error| error.to_string())?;

    Ok(Some(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )))
}

#[tauri::command]
pub fn ai_list_endpoints(
    runtime: State<'_, AiRuntime>,
) -> Result<Vec<crate::ai::remote::types::EndpointInfo>, String> {
    Ok(runtime
        .endpoints()
        .list()
        .into_iter()
        .map(|e| crate::ai::remote::types::EndpointInfo {
            id: e.id,
            name: e.name,
            base_url: e.base_url,
            models: e.models,
            disabled_models: e.disabled_models,
            enabled: e.enabled,
            has_api_key: !e.api_key.trim().is_empty(),
        })
        .collect())
}

#[tauri::command]
pub fn ai_add_endpoint(
    runtime: State<'_, AiRuntime>,
    endpoint: crate::ai::remote::types::Endpoint,
) -> Result<(), String> {
    runtime.endpoints().add(endpoint)
}

#[tauri::command]
pub fn ai_update_endpoint(
    runtime: State<'_, AiRuntime>,
    mut endpoint: crate::ai::remote::types::Endpoint,
) -> Result<(), String> {
    if endpoint.api_key.trim().is_empty() {
        endpoint.api_key = runtime
            .endpoints()
            .get(&endpoint.id)
            .map(|existing| existing.api_key)
            .unwrap_or_default();
    }
    if runtime.endpoints().update(endpoint)? {
        Ok(())
    } else {
        Err("Provider not found".to_string())
    }
}

/// Issues a GET for a provider's model list.
///
/// `key_in_query` carries the credential in the query string instead of an
/// `Authorization` header. OpenAI-compatible endpoints use the header, but a
/// provider's native API may only read `?key=`, so both are supported and the
/// caller retries once when the first attempt is rejected on credentials.
async fn send(
    client: &reqwest::Client,
    url: &str,
    api_key: &str,
    key_in_query: bool,
) -> Result<reqwest::Response, String> {
    let mut request = client.get(url);
    if !api_key.trim().is_empty() {
        request = if key_in_query {
            request.query(&[("key", api_key)])
        } else {
            request.bearer_auth(api_key)
        };
    }
    request
        .send()
        .await
        .map_err(|error| format!("Connection failed: {error}"))
}

/// Full `/models` payload, kept so the caller can learn capabilities and not
/// just the ids.
///
/// A bare OpenAI provider returns ids with nothing else, which is a successful
/// answer with no capabilities in it — not a failure.
async fn fetch_models_with_capabilities(
    endpoint: &crate::ai::remote::types::Endpoint,
) -> Result<Vec<(String, crate::ai::remote::capabilities::ModelCapabilities)>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|error| error.to_string())?;
    let url = crate::ai::remote::types::models_url(&endpoint.base_url);
    let response = send(&client, &url, &endpoint.api_key, false).await?;
    let response = match response.status() {
        // Some providers only accept the key as a query parameter and ignore an
        // `Authorization` header, so the request succeeds in shape but fails on
        // credentials. One retry with the key in the query string settles it,
        // and the status that triggers it is narrow enough not to mask a real
        // rejection such as an invalid key.
        reqwest::StatusCode::UNAUTHORIZED
        | reqwest::StatusCode::FORBIDDEN
        | reqwest::StatusCode::BAD_REQUEST => send(&client, &url, &endpoint.api_key, true).await?,
        _ => response,
    };
    if !response.status().is_success() {
        // A provider may expose an OpenAI-compatible chat endpoint without
        // implementing a model listing on the same path; Google's compatibility
        // layer is one. That is not a broken provider, so the message says what
        // still works rather than reporting a failure the user cannot act on.
        return Err(if response.status() == reqwest::StatusCode::NOT_FOUND {
            "This provider does not list models at that address. Chat may still work — add model names by hand.".to_string()
        } else {
            format!("Provider returned HTTP {}", response.status())
        });
    }
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|error| format!("Invalid model response: {error}"))?;
    let models = crate::ai::remote::capabilities::parse_models(&body);
    if models.is_empty() {
        return Err(
            "Provider answered, but its model list was empty or in an unreadable shape."
                .to_string(),
        );
    }
    Ok(models)
}

/// What one model of a provider is known to support.
///
/// Every field is optional and `null` means the provider never said. The
/// frontend falls back to its previous behaviour when a field is unknown, which
/// is what keeps an unrecognised provider working unchanged.
#[tauri::command]
pub fn ai_model_capabilities(
    runtime: State<'_, AiRuntime>,
    endpoint_id: String,
    model: String,
) -> Result<crate::ai::remote::capabilities::ModelCapabilities, String> {
    runtime.database().model_capabilities(&endpoint_id, &model)
}

/// Turns one remote model on or off in the pickers.
///
/// Writes `provider_models.enabled`, which `ai_list_endpoints` filters on, so
/// the next read of the provider list already excludes it. No frontend-side
/// filtering is involved: a model that is off is absent, not present-and-flagged.
#[tauri::command]
pub fn ai_set_model_enabled(
    runtime: State<'_, AiRuntime>,
    endpoint_id: String,
    model: String,
    enabled: bool,
) -> Result<(), String> {
    runtime
        .database()
        .set_model_enabled(&endpoint_id, &model, enabled)
}

/// Every known model of one provider, keyed by model id.
///
/// A picker shows a whole provider's list, so this answers in one call rather
/// than one per model: kilocode reports several hundred models, and firing a
/// command for each of them at once floods the command queue. One read per
/// provider is also simply cheaper.
#[tauri::command]
pub fn ai_provider_capabilities(
    runtime: State<'_, AiRuntime>,
    endpoint_id: String,
) -> Result<
    std::collections::HashMap<String, crate::ai::remote::capabilities::ModelCapabilities>,
    String,
> {
    runtime.database().provider_capabilities(&endpoint_id)
}

#[tauri::command]
pub async fn ai_test_endpoint(
    runtime: State<'_, AiRuntime>,
    mut endpoint: crate::ai::remote::types::Endpoint,
) -> Result<Vec<String>, String> {
    if endpoint.api_key.trim().is_empty() {
        endpoint.api_key = runtime
            .endpoints()
            .get(&endpoint.id)
            .map(|existing| existing.api_key)
            .unwrap_or_default();
    }
    let models = fetch_models_with_capabilities(&endpoint).await?;
    // The list just read is the provider's own current catalogue, so this is
    // where the stored rows are brought in line with it: models it has dropped
    // are removed, ones it has gained are added switched off. Done before the
    // capabilities write so every row that write touches already exists, and
    // best-effort like that write -- a bookkeeping failure must not fail a
    // connection test that has already succeeded.
    let model_ids: Vec<String> = models.iter().map(|(model, _)| model.clone()).collect();
    let _ = runtime
        .database()
        .reconcile_provider_models(&endpoint.id, &model_ids);
    // Testing a provider is the one moment we know its answers are current, so
    // this is where capabilities are learned. A provider that reports none is
    // stored as unknown, and the app keeps behaving as it did before. The stamp
    // is only a record of when we last asked, so seconds since the epoch is
    // enough and avoids pulling in a date library for one value.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let now = now.to_string();
    let _ = runtime
        .database()
        .save_model_capabilities(&endpoint.id, &models, &now);
    Ok(models.into_iter().map(|(model, _)| model).collect())
}

#[tauri::command]
pub fn ai_remove_endpoint(runtime: State<'_, AiRuntime>, id: String) -> Result<(), String> {
    runtime.endpoints().remove(&id)
}

#[tauri::command]
pub fn local_runtime_settings(
    runtime: State<'_, AiRuntime>,
    model_id: String,
) -> crate::ai::local::LocalRuntimeSettings {
    runtime.local_models().runtime_settings(&model_id)
}

/// Estimated VRAM for one model under one set of runtime settings.
///
/// Takes the settings as an argument rather than reading the stored ones, because
/// the caller is dragging a slider: the figure has to track the value under the
/// thumb before anything is saved, or the estimate only moves after the setting
/// is committed and stops answering the question the drag is asking.
#[tauri::command]
pub fn local_runtime_estimate(
    runtime: State<'_, AiRuntime>,
    model_id: String,
    settings: crate::ai::local::LocalRuntimeSettings,
) -> Result<crate::ai::local::memory::MemoryEstimate, String> {
    let Some(model) = runtime.local_models().model(&model_id) else {
        return Err("That local model is no longer registered".to_string());
    };
    let device = crate::ai::local::device::DeviceMemory::local();
    let mut estimate = crate::ai::local::memory::estimate(
        std::path::Path::new(&model.path),
        settings
            .mmproj
            .as_deref()
            .filter(|path| !path.trim().is_empty())
            .map(std::path::Path::new),
        &settings,
        device.map(|value| value.total_bytes),
    );
    // Computed here rather than in the frontend, so the one-directional rule --
    // it may say the estimate does not fit, and never that it does -- has a single
    // owner. A UI that decided for itself could report the opposite.
    estimate.exceeds_device = estimate.exceeds_device();
    Ok(estimate)
}

/// What the local GPU reports, when this build can ask it.
///
/// Separate from the estimate so the UI can say *why* a fit verdict is missing
/// rather than silently omitting it.
#[tauri::command]
pub fn local_device_memory() -> Option<crate::ai::local::device::DeviceMemory> {
    crate::ai::local::device::DeviceMemory::local().cloned()
}

/// Every KV-cache quantisation `llama-server` accepts.
///
/// Asked of the backend rather than spelled out in the frontend, because the list
/// has to match what the binary accepts: a UI with its own copy would eventually
/// offer one the binary rejects, and that fails at spawn as a server that will
/// not start rather than in the settings page where the mistake was made.
#[tauri::command]
pub fn local_kv_quantisations() -> Vec<String> {
    crate::ai::local::KvQuant::ALL
        .iter()
        .map(|quant| quant.as_flag().to_string())
        .collect()
}

/// Whether a path the user chose is still there.
///
/// A model or projector file can be moved, renamed or deleted outside the app,
/// and a settings page that still shows the old path as configured is a page
/// lying about the state of the thing it configures.
///
/// **The `\\?\` prefix is stripped first.** Windows file dialogs and
/// `std::fs::canonicalize` return device paths -- `\\?\F:\models\a.gguf` -- and
/// a string comparison against that form fails for a file that is plainly there.
/// The prefix exists to bypass path parsing for long paths, and the parser
/// handles them fine on current Windows, so removing it is safe and makes the
/// check agree with what the dialog showed.
///
/// Deliberately just an existence check and nothing more: this is asked of a path
/// the user picked through the app's own dialog, never of arbitrary input, so it
/// grants no read access the dialog did not already grant.
#[tauri::command]
pub fn local_path_exists(path: String) -> bool {
    crate::ai::local::memory::normalise_windows_path(&path).is_file()
}

#[tauri::command]
pub fn local_runtime_settings_save(
    runtime: State<'_, AiRuntime>,
    model_id: String,
    settings: crate::ai::local::LocalRuntimeSettings,
) -> Result<crate::ai::local::LocalRuntimeSettings, String> {
    runtime
        .local_models()
        .save_runtime_settings(&model_id, settings)
}

#[tauri::command]
pub async fn local_engines_list(
    app: AppHandle,
    refresh: bool,
) -> Result<Vec<crate::ai::local::engines::EngineInfo>, String> {
    crate::ai::local::engines::list(&app, refresh).await
}

#[tauri::command]
pub async fn local_engines_install(app: AppHandle, id: String) -> Result<(), String> {
    crate::ai::local::engines::install(app, &id).await
}

#[tauri::command]
pub fn local_engines_uninstall(app: AppHandle, id: String) -> Result<(), String> {
    crate::ai::local::engines::uninstall(&app, &id)
}

/// Install an engine from a GitHub release link.
///
/// The route for a fork: a build this app's own catalog does not carry, because
/// it is published by another owner, at a tag that is not a `bNNNN` build
/// number. Verified against the catalog's checks in every respect except the
/// checksum -- a download link carries no digest, so there is nothing to check
/// the bytes against and the engine is labelled rather than silently trusted.
#[tauri::command]
pub async fn local_engines_custom_install(app: AppHandle, link: String) -> Result<(), String> {
    crate::ai::local::engines::custom_install(app, &link).await
}

/// Set which engine a local model runs with.
///
/// Every model carries its own engine, so this is the only write path -- there
/// is no global default to fall back to. The engine has to be installed first,
/// so a choice can only ever name files that are on disk, and the running model
/// is unloaded when its engine changes, because it is being served by the old
/// one right now.
#[tauri::command]
pub fn local_model_engine_set(
    app: AppHandle,
    runtime: State<'_, AiRuntime>,
    model_id: String,
    engine_id: String,
) -> Result<(), String> {
    crate::ai::local::engines::require_installed(&app, &engine_id)?;
    runtime
        .local_models()
        .set_model_engine(&model_id, engine_id)?;
    runtime.local_models().unload_if_loaded(&model_id);
    Ok(())
}

#[tauri::command]
pub fn local_models_list(runtime: State<'_, AiRuntime>) -> Vec<crate::ai::local::LocalModel> {
    runtime.local_models().list()
}

#[tauri::command]
pub fn local_models_add(
    runtime: State<'_, AiRuntime>,
    path: String,
) -> Result<crate::ai::local::LocalModel, String> {
    runtime.local_models().add(path)
}

#[tauri::command]
pub fn local_models_remove(runtime: State<'_, AiRuntime>, id: String) -> Result<(), String> {
    runtime.local_models().remove(&id)
}

#[tauri::command]
pub fn local_model_select(runtime: State<'_, AiRuntime>, id: String) -> Result<(), String> {
    runtime.local_models().select(&id)
}

#[tauri::command]
pub fn local_model_status(runtime: State<'_, AiRuntime>) -> crate::ai::local::LocalRuntimeStatus {
    runtime.local_models().status()
}

#[tauri::command]
pub fn local_model_logs(runtime: State<'_, AiRuntime>) -> Vec<String> {
    runtime.local_models().server_logs()
}

#[tauri::command]
pub fn local_model_load(
    app: AppHandle,
    runtime: State<'_, AiRuntime>,
    id: String,
) -> Result<(), String> {
    app.emit(
        "local-model-event",
        serde_json::json!({"kind": "loading", "modelId": id}),
    )
    .map_err(|error| error.to_string())?;
    match runtime.local_models().load(app.clone(), id.clone()) {
        Ok(()) => {
            // Asked for from the interface, so a scheduled run must leave it alone
            // from here on -- even if a job was the one that started it.
            runtime.local_models().mark_loaded_for_user();
            Ok(())
        }
        Err(error) => {
            let _ = app.emit(
                "local-model-event",
                serde_json::json!({"kind": "failed", "modelId": id, "message": error}),
            );
            Err(error)
        }
    }
}

#[tauri::command]
pub fn local_model_unload(runtime: State<'_, AiRuntime>) {
    runtime.local_models().unload();
}
