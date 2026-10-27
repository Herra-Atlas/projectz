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

/// Adds each reply's record of its tools to its text, before a send.
///
/// The chat view used to do this in TypeScript; a job and a message from a phone had
/// no copy, so they remembered what they said and not what they saw. One
/// implementation means the bytes are identical whichever door the run came through,
/// which is exactly what the provider's cached prefix needs.
#[tauri::command]
pub fn ai_recap_messages(messages: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
    messages
        .into_iter()
        .map(|mut message| {
            let recap = {
                let content = message
                    .get("content")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                match message.get("activity") {
                    Some(activity) => crate::ai::recap::append(content, activity),
                    None => content.to_string(),
                }
            };
            if let Some(object) = message.as_object_mut() {
                object.insert("content".into(), serde_json::Value::String(recap));
            }
            message
        })
        .collect()
}

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

/// Compacts a stored conversation now, at the user's request -- the composer's `/compact`.
///
/// **The stored transcript, not a request copy.** The automatic pass in
/// `ai::remote::client` trims what one reply sends and leaves the database alone; this is
/// the other door -- a user who knows the conversation is too long asks for the same trim
/// to be written down, so the rows they see are what the model will be sent. It applies the
/// same elision, at the same mode, and leaves a hidden notice in the transcript the way the
/// automatic pass leaves one in the run.
///
/// Returns the messages to draw, or `Err` with a reason the composer can show plainly.
#[tauri::command]
pub async fn ai_compact_session(
    database: State<'_, std::sync::Arc<crate::database::Database>>,
    session_id: String,
) -> Result<Vec<crate::ai::types::ChatMessage>, String> {
    // The knobs are the stored preference, read here rather than passed in: a
    // command is not a second place a setting lives, and the user asking to
    // compact means "do what you would do", not "do something the next reply
    // will not".
    let settings = crate::ai::compact::stored(database.as_ref());
    match crate::ai::compact::compact_session(database.as_ref(), &session_id, settings)? {
        // Nothing old enough to elide. The composer says so rather than reporting a
        // compaction that did not happen.
        None => Ok(Vec::new()),
        Some(messages) => {
            info!(session_id = %session_id, count = messages.len(), "conversation compacted");
            Ok(messages
                .into_iter()
                .filter_map(|value| serde_json::from_value(value).ok())
                .collect())
        }
    }
}

/// Re-points the level a reply in flight decides at.
///
/// The composer stays live while a reply streams, so a level picked there has to
/// reach the run that is already going -- otherwise the screen shows one level
/// while the run uses another, and a level that "did not work" is really a level
/// that arrived a message too late. Returns whether a run with that id was going;
/// `false` is normal (the reply just ended) and harmless, because the next send
/// carries the level regardless.
#[tauri::command]
pub fn ai_set_run_permission(
    runtime: State<'_, AiRuntime>,
    run_id: String,
    permission: crate::ai::tools::PermissionMode,
) -> bool {
    runtime.set_run_permission(&run_id, permission)
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

/// Answers a pending `ask_user` question with the user's reply text.
///
/// Returns whether a question was actually waiting, on the same reasoning as
/// [`ai_answer_approval`]: `false` is a stale answer from a prompt whose run has
/// already ended, which is normal and not an error to retry.
#[tauri::command]
pub fn ai_answer_question(
    runtime: State<'_, AiRuntime>,
    question_id: String,
    answer: String,
) -> bool {
    runtime.answer_question(&question_id, answer)
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

/// Everything the Taste settings page draws.
#[tauri::command]
pub fn taste_status(taste: State<'_, crate::taste::Taste>) -> crate::taste::TasteStatus {
    taste.status()
}

/// Writes the taste record and returns what was stored.
#[tauri::command]
pub fn taste_save_settings(
    app: AppHandle,
    taste: State<'_, crate::taste::Taste>,
    settings: crate::taste::TasteSettings,
) -> Result<crate::taste::TasteSettings, String> {
    taste.save(&app, &settings)
}

/// Runs a learning pass now, whether or not the schedule says so.
#[tauri::command]
pub async fn taste_run_now(
    app: AppHandle,
    runtime: State<'_, AiRuntime>,
    taste: State<'_, crate::taste::Taste>,
) -> Result<crate::taste::TasteRunView, String> {
    taste.run_now(&app, &runtime).await
}

/// The last runs, newest first.
#[tauri::command]
pub fn taste_runs(
    taste: State<'_, crate::taste::Taste>,
    limit: Option<i64>,
) -> Vec<crate::taste::TasteRunView> {
    taste.runs(limit.unwrap_or(20))
}

/// Rewrites one learned fact.
#[tauri::command]
pub fn taste_update_item(
    app: AppHandle,
    taste: State<'_, crate::taste::Taste>,
    id: i64,
    text: String,
) -> Result<crate::taste::TasteStatus, String> {
    taste.update_item(&app, id, &text)
}

/// Deletes one learned fact.
#[tauri::command]
pub fn taste_delete_item(
    app: AppHandle,
    taste: State<'_, crate::taste::Taste>,
    id: i64,
) -> Result<crate::taste::TasteStatus, String> {
    taste.delete_item(&app, id)
}

/// Everything the Communications page draws: the record and each channel's state.
///
/// One read rather than one per channel, so a page with two transports cannot show
/// one of them a second out of date.
#[tauri::command]
pub fn messaging_status(
    messaging: State<'_, std::sync::Arc<crate::messaging::Messaging>>,
) -> crate::messaging::MessagingStatus {
    messaging.status()
}

/// Writes the shared record and answers with what was stored.
///
/// The record is clamped by the backend (the allowlist is numeric only, `jobs` is
/// always off), so the page settles on the stored values rather than on what was
/// asked for.
#[tauri::command]
pub fn messaging_save_settings(
    app: AppHandle,
    messaging: State<'_, std::sync::Arc<crate::messaging::Messaging>>,
    settings: crate::messaging::settings::MessagingSettings,
) -> Result<crate::messaging::settings::MessagingSettings, String> {
    let saved = messaging.save(settings)?;
    let _ = app.emit("messaging-event", serde_json::json!({ "kind": "state" }));
    Ok(saved)
}

/// Moves the master switch. Every channel parks or wakes from this one fact.
#[tauri::command]
pub fn messaging_set_enabled(
    app: AppHandle,
    messaging: State<'_, std::sync::Arc<crate::messaging::Messaging>>,
    enabled: bool,
) -> Result<crate::messaging::MessagingStatus, String> {
    messaging.set_enabled(enabled)?;
    let _ = app.emit("messaging-event", serde_json::json!({ "kind": "state" }));
    Ok(messaging.status())
}

/// Writes one channel's own configuration (Telegram's token, today).
///
/// The channel validates it before anything is stored, so a mistyped token is
/// reported at the field and the previous one keeps working.
#[tauri::command]
pub async fn messaging_save_channel(
    app: AppHandle,
    messaging: State<'_, std::sync::Arc<crate::messaging::Messaging>>,
    channel: String,
    config: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let hub = std::sync::Arc::clone(messaging.inner());
    let Some(target) = hub.channel(&channel) else {
        return Err(format!("There is no {channel} channel in this build"));
    };
    let view = target.configure(config).await?;
    // A poller parked because it had no token must not wait out its idle timer
    // before noticing that it now has one.
    hub.wake();
    let _ = app.emit("messaging-event", serde_json::json!({ "kind": "state" }));
    Ok(view)
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
    // The taste tables exist only once the migration has run, so the prompt cache
    // primed during setup could not have read them. Prime it again now.
    crate::taste::refresh_prompt_cache(&runtime.database());
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

/// The installation-wide tool switches.
#[tauri::command]
pub fn host_access_get(runtime: State<'_, AiRuntime>) -> crate::ai::tools::AccessSet {
    crate::ai::tools::host::load(runtime.database().as_ref())
}

/// Replaces the installation-wide tool switches.
///
/// Takes effect on the next gate a run is built with. A run already in flight
/// keeps the access it started under, which is the same rule its permission level
/// follows.
#[tauri::command]
pub fn host_access_save(
    runtime: State<'_, AiRuntime>,
    access: crate::ai::tools::AccessSet,
) -> Result<crate::ai::tools::AccessSet, String> {
    crate::ai::tools::host::save(runtime.database().as_ref(), access)
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

/// Repaints the native window frame for the theme the frontend resolved.
///
/// Every surface inside the webview reads a CSS token, but the title bar and its
/// buttons are drawn by the OS, so matching them means telling the window.
/// `system` -- or any value not recognised -- hands the frame back to the
/// operating system, which is the one case where following it is correct.
#[tauri::command]
pub fn set_window_theme(window: tauri::WebviewWindow, theme: Option<String>) -> Result<(), String> {
    window
        .set_theme(crate::window::frame_theme(theme.as_deref()))
        .map_err(|error| error.to_string())
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

/// Every configured MCP server, with what each said last time it answered.
///
/// Reads the last result rather than connecting, so the page paints immediately
/// and a server that hangs cannot hold up the settings window. [`mcp_refresh`] is
/// what goes and asks.
#[tauri::command]
pub fn mcp_servers(runtime: State<'_, AiRuntime>) -> Vec<crate::mcp::McpServerView> {
    crate::mcp::manager::views(&runtime.database())
}

/// Connects to every enabled server and returns the refreshed rows.
#[tauri::command]
pub async fn mcp_refresh(
    runtime: State<'_, AiRuntime>,
) -> Result<Vec<crate::mcp::McpServerView>, String> {
    let database = runtime.database();
    Ok(crate::mcp::manager::refresh(&database).await)
}

/// Adds a server, or replaces one by id, then reaches it.
///
/// The refresh is part of the write rather than something the caller remembers to
/// do after: a row saved and not reached would show no tools and no error, which
/// reads as a server that works and offers nothing.
#[tauri::command]
pub async fn mcp_save(
    runtime: State<'_, AiRuntime>,
    server: crate::mcp::McpServer,
) -> Result<Vec<crate::mcp::McpServerView>, String> {
    let database = runtime.database();
    let mut servers = crate::mcp::load(&database);
    let mut saved = server;
    if saved.id.trim().is_empty() {
        saved.id = crate::mcp::new_id();
    }
    match servers.iter_mut().find(|existing| existing.id == saved.id) {
        Some(existing) => *existing = saved.clone(),
        None => servers.push(saved.clone()),
    }
    crate::mcp::save(&database, &servers)?;
    // A saved server may have a new command, so whatever was connected is gone
    // and the refresh below starts it again.
    crate::mcp::manager::forget(&saved.id).await;
    Ok(crate::mcp::manager::refresh(&database).await)
}

/// Removes a server and stops it if it was running.
#[tauri::command]
pub async fn mcp_delete(
    runtime: State<'_, AiRuntime>,
    id: String,
) -> Result<Vec<crate::mcp::McpServerView>, String> {
    let database = runtime.database();
    let servers = crate::mcp::load(&database)
        .into_iter()
        .filter(|server| server.id != id)
        .collect::<Vec<_>>();
    crate::mcp::save(&database, &servers)?;
    crate::mcp::manager::forget(&id).await;
    Ok(crate::mcp::manager::refresh(&database).await)
}

/// Every skill folder. For the management screen.
///
/// No database and no filter: a skill is its folder, so there is no "disabled"
/// set to exclude and nothing the page has not already seen on disk.
#[tauri::command]
pub fn skills_list() -> Result<Vec<crate::skills::Skill>, String> {
    crate::skills::list()
}

/// One skill's body and the files beside it.
#[tauri::command]
pub fn skills_get(
    slug: String,
) -> Result<(crate::skills::Skill, String, Vec<crate::skills::SkillFile>), String> {
    crate::skills::get(&slug)
}

/// Creates a skill folder. The slug and the header are written here rather than
/// accepted from the frontend, so the folder and the `name` in it cannot disagree.
#[tauri::command]
pub fn skills_create(skill: crate::skills::SkillDraft) -> Result<crate::skills::Skill, String> {
    crate::skills::create(&skill)
}

/// Replaces a skill's text, keeping its slug and files.
#[tauri::command]
pub fn skills_update(
    slug: String,
    skill: crate::skills::SkillDraft,
) -> Result<crate::skills::Skill, String> {
    crate::skills::update(&slug, &skill)
}

#[tauri::command]
pub fn skills_delete(slug: String) -> Result<(), String> {
    crate::skills::delete(&slug)
}

/// Writes a text file into a skill folder.
///
/// Kept for the agent's own door and for anything scripting the app; the Skills
/// page itself edits a folder through the Files panel, which has its own commands
/// and lands in the same place.
#[tauri::command]
pub fn skills_write_file(
    slug: String,
    path: String,
    content: String,
) -> Result<crate::skills::SkillFile, String> {
    crate::skills::write_file(&slug, &path, &content)
}

#[tauri::command]
pub fn skills_read_file(slug: String, path: String) -> Result<String, String> {
    crate::skills::read_file(&slug, &path)
}

#[tauri::command]
pub fn skills_delete_file(slug: String, path: String) -> Result<(), String> {
    crate::skills::delete_file(&slug, &path)
}

/// The folder behind a skill, so the page can browse it in the Files panel.
#[tauri::command]
pub fn skills_folder(slug: String) -> Result<String, String> {
    let skill = crate::skills::resolve(&slug)?;
    Ok(crate::skills::directory_of(&skill)?.display().to_string())
}

/// Installs a skill from a `.zip` link or a repository that holds exactly one.
///
/// User-initiated by construction: this is a command the Settings button calls,
/// never a tool the model can reach. See `skills::install` for why.
#[tauri::command]
pub async fn skills_install(url: String) -> Result<crate::skills::Skill, String> {
    crate::skills::install::install_from_url(&url).await
}

/// Resolves a GitHub repository link to the skills it publishes.
///
/// One tree request, not one per folder: the picker needs the list before the
/// user chooses anything, and the API quota is small enough that spending it on
/// discovery would make a seven-skill repository cost more than the skills.
#[tauri::command]
pub async fn skills_discover(
    app: AppHandle,
    url: String,
) -> Result<crate::skills::github::Discovery, String> {
    let progress = move |step: crate::skills::github::Progress| {
        let _ = app.emit("skills-install", step);
    };
    crate::skills::github::discover_url(&url, &progress).await
}

/// Installs the folders the picker selected.
///
/// Progress rides on `skills-install` events between the first byte and the
/// last row, so the dialog can show a bar rather than a spinner that says
/// nothing about whether anything is happening.
#[tauri::command]
pub async fn skills_install_selected(
    app: AppHandle,
    url: String,
    reference: String,
    skills: Vec<String>,
) -> Result<crate::skills::github::InstallReport, String> {
    let progress = move |step: crate::skills::github::Progress| {
        let _ = app.emit("skills-install", step);
    };
    crate::skills::github::install_url_selected(&url, &reference, &skills, &progress).await
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
pub fn job_scheduler_status(scheduler: State<'_, crate::jobs::Scheduler>) -> serde_json::Value {
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

/// The host an icon is looked for on, for a URL.
///
/// Only the parse and the refusals live here: *which* host actually holds the
/// icon is [`icon_hosts`]'s question, and it answers it the same way for both
/// lists. This used to strip an `api.` prefix, which worked for the provider list
/// by coincidence and not at all for an MCP server reached at `mcp.…` — a rule
/// that named one prefix rather than describing the shape the two share.
fn icon_host(url: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(url).map_err(|_| "Invalid URL".to_string())?;
    if url.scheme() != "https" {
        return Err("Icons require an HTTPS URL".to_string());
    }
    let host = url
        .host_str()
        .ok_or_else(|| "URL has no hostname".to_string())?;
    if host.parse::<IpAddr>().is_ok() || host.is_empty() {
        return Err("Icons require a public hostname".to_string());
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

/// The most an icon may weigh, so a settings page cannot be made to download
/// something enormous by a URL typed into it.
const MAX_ICON_BYTES: usize = 256 * 1024;

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
    let host = icon_host(&base_url)?;
    icon_for_host(&app, &host).await
}

/// The icon an MCP server's command line points at, when it names a URL.
///
/// The same store as the provider icons -- one cache directory, one fetch, one
/// set of rules for what counts as an image -- because an icon is the same thing
/// in both lists. A server that is a program has nothing to fetch, which is most
/// of them, so no URL is `None` rather than an error: the row falls back to a
/// glyph, and that is the ordinary case rather than a failure.
#[tauri::command]
pub async fn mcp_favicon(app: AppHandle, command: String) -> Result<Option<String>, String> {
    let Some(url) = crate::mcp::url_in(&command) else {
        return Ok(None);
    };
    let host = icon_host(url)?;
    icon_for_host(&app, &host).await
}

/// Fetches and caches the icon for a host, climbing to the site it belongs to.
///
/// Read from disk when it is already there, so a second look at a list costs
/// nothing, and written through a temporary file that is renamed into place, so
/// a half-written icon is never read back as a whole one. What is written is
/// stored under the host that was **asked** for, not the one it was found on, so
/// the climb costs one round of requests rather than one per render.
async fn icon_for_host(app: &AppHandle, host: &str) -> Result<Option<String>, String> {
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

    for candidate in icon_hosts(host) {
        let Some(bytes) = fetch_icon(&candidate).await? else {
            continue;
        };
        let Some(mime) = favicon_mime(&bytes) else {
            continue;
        };
        write_icon(&cache_path, &bytes)?;
        return Ok(Some(favicon_data_url(&bytes, mime)));
    }
    Ok(None)
}

/// The hosts to try for one icon, most specific first.
///
/// **A service is often reached at a subdomain that serves nothing.** The MCP
/// endpoint is `mcp.deepwiki.com` and the site, with the icon, is `deepwiki.com`;
/// a provider's API is `api.openai.com` and its brand is `openai.com`. Those are
/// the same fact, so this is the same rule: climb one label at a time and stop
/// while a domain is still left, because one more step would ask about a TLD.
/// That is why there is no list of known prefixes here to grow stale -- and why
/// the provider list and the MCP list behave identically, since this is the only
/// place either of them looks.
fn icon_hosts(host: &str) -> Vec<String> {
    let mut hosts = vec![host.to_string()];
    let mut current = host.to_string();
    while current.split('.').count() > 2 {
        let Some((_, parent)) = current.split_once('.') else {
            break;
        };
        if parent.is_empty() {
            break;
        }
        hosts.push(parent.to_string());
        current = parent.to_string();
    }
    hosts
}

/// Downloads one `/favicon.ico` and returns what came back, if anything did.
///
/// The address is resolved here and pinned to that answer, and a host that is not
/// publicly routable is refused without a request: a settings page has no business
/// being able to probe the local network by being typed into.
async fn fetch_icon(host: &str) -> Result<Option<Vec<u8>>, String> {
    let addresses: Vec<SocketAddr> = tokio::net::lookup_host((host, 443))
        .await
        .map_err(|_| "The hostname could not be resolved".to_string())?
        .collect();
    if addresses.is_empty() || addresses.iter().any(|address| !is_public_ip(address.ip())) {
        return Ok(None);
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        // `reqwest` sends no `User-Agent` unless told to, and some hosts answer a
        // request without one by refusing it -- which reads here as "no icon" and
        // is impossible to tell apart from a host that genuinely has none. Naming
        // the app is also the honest thing to do when asking someone's server.
        .user_agent(concat!("projectz/", env!("CARGO_PKG_VERSION")))
        .resolve_to_addrs(host, &addresses)
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
    Ok(Some(bytes))
}

/// Writes an icon through a temporary file that is renamed into place.
fn write_icon(cache_path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary_path = cache_path.with_extension(format!(
        "{}.{}.tmp",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    fs::write(&temporary_path, bytes).map_err(|error| error.to_string())?;
    if cache_path.exists() {
        fs::remove_file(cache_path).map_err(|error| error.to_string())?;
    }
    fs::rename(&temporary_path, cache_path).map_err(|error| error.to_string())
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
    mut settings: crate::ai::local::LocalRuntimeSettings,
) -> Result<crate::ai::local::LocalRuntimeSettings, String> {
    // The app has to know where its server is and which model it loaded, so a
    // custom list may not touch those four flags. Caught here rather than when the
    // server fails to answer, where the cause would be a mystery.
    if let Some(custom) = settings.custom_args.as_deref() {
        if custom.trim().is_empty() {
            settings.custom_args = None;
        } else if let Some(argument) = settings
            .effective_args()
            .into_iter()
            .find(|argument| crate::ai::local::RESERVED_ARGUMENTS.contains(&argument.as_str()))
        {
            return Err(format!(
                "{argument} is set by the app, so it cannot appear in the custom arguments."
            ));
        }
    }
    runtime
        .local_models()
        .save_runtime_settings(&model_id, settings)
}

/// The argument list these settings produce, so the page can show it as one line.
///
/// Takes the settings rather than a model id: the box has to show what the form
/// currently says, not what was last saved.
#[tauri::command]
pub fn local_runtime_arguments(settings: crate::ai::local::LocalRuntimeSettings) -> String {
    crate::ai::local::quote_arguments(&settings.to_args())
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
pub fn local_engines_uninstall(app: AppHandle, id: String) -> Result<Option<String>, String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The case this whole walk exists for: an endpoint on a subdomain, whose
    /// icon is on the site underneath it.
    #[test]
    fn the_climb_reaches_the_site_under_a_subdomain() {
        assert_eq!(
            icon_hosts("mcp.deepwiki.com"),
            vec!["mcp.deepwiki.com", "deepwiki.com"]
        );
        // The provider case, which used to need its own rule for `api.`.
        assert_eq!(
            icon_hosts("api.openai.com"),
            vec!["api.openai.com", "openai.com"]
        );
    }

    #[test]
    fn a_deeper_subdomain_climbs_one_label_at_a_time() {
        assert_eq!(
            icon_hosts("a.b.example.com"),
            vec!["a.b.example.com", "b.example.com", "example.com"]
        );
    }

    /// One more step would ask about a TLD, which is never where an icon is.
    #[test]
    fn the_climb_stops_at_a_domain() {
        assert_eq!(icon_hosts("example.com"), vec!["example.com"]);
        assert_eq!(icon_hosts("localhost"), vec!["localhost"]);
    }

    #[test]
    fn a_host_is_read_from_a_url_and_only_an_https_one() {
        assert_eq!(
            icon_host("https://mcp.deepwiki.com/mcp").expect("host"),
            "mcp.deepwiki.com"
        );
        assert!(icon_host("http://mcp.deepwiki.com/mcp").is_err());
        assert!(icon_host("https://127.0.0.1:8931/mcp").is_err());
        assert!(icon_host("not a url").is_err());
    }
}
