use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex},
};

use tauri::{AppHandle, Emitter};
use tracing::info;

use crate::ai::local::LocalModelManager;
use crate::ai::remote::client::stream_chat;
use crate::ai::remote::registry::EndpointRegistry;
use crate::ai::types::{ChatEvent, ChatRequest, ChatResult};

/// Output cap for a title request. A title is at most six words, so a small
/// ceiling stops a reasoning model spending its budget deliberating about
/// something nobody will read.
const TITLE_MAX_TOKENS: u32 = 24;

/// Tries allowed per title model before moving to the next one. A retry covers
/// a transient error or one unusable answer without giving the model up, while
/// a model that is consistently wrong still fails fast.
const TITLE_ATTEMPTS: u32 = 2;

/// One sub-agent at work, as the panel needs to know it.
///
/// The same facts the start event carries, held so the panel can list an agent
/// that began before it was opened. Field names are camelCase for the frontend,
/// matching the session headers the panel already reads.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunningSubAgent {
    pub id: String,
    pub label: String,
    pub session_id: Option<String>,
    pub prompt: String,
    pub model: String,
    pub provider: String,
    pub started_at: String,
}

#[derive(Clone)]
pub struct AiRuntime {
    endpoints: Arc<EndpointRegistry>,
    local_models: Arc<LocalModelManager>,
    database: Arc<crate::database::Database>,
    active_runs: Arc<Mutex<HashMap<String, Arc<std::sync::atomic::AtomicBool>>>>,
    local_runs: Arc<Mutex<HashSet<String>>>,
    local_completions: Arc<Mutex<HashMap<String, String>>>,
    /// The session the user is currently looking at, set by the frontend on every
    /// session change. A completion for this session is already on screen, so it must
    /// not raise a notification.
    viewed_session: Arc<Mutex<Option<String>>>,
    /// The sub-agents running right now, keyed by their run id.
    ///
    /// A run is only written to the database when it finishes, so a panel that
    /// read only storage would be blind to an agent already at work. Held in
    /// memory because that is exactly the window storage cannot cover: an entry
    /// is added when a sub-agent starts and removed when it ends, and a process
    /// that dies takes its stale entries with it.
    running_subagents: Arc<Mutex<HashMap<String, RunningSubAgent>>>,
    /// The permission gate for tool calls.
    ///
    /// Owned by the runtime rather than built per run because the answer to a
    /// prompt arrives as a separate command that has no reference to the run
    /// that asked. One gate means one place holds the outstanding prompts.
    approval: crate::ai::tools::ApprovalGate,
}

impl AiRuntime {
    pub fn new_with_database(
        engine_dir: PathBuf,
        database: Arc<crate::database::Database>,
    ) -> Self {
        Self {
            endpoints: Arc::new(EndpointRegistry::new(Arc::clone(&database))),
            local_models: Arc::new(
                LocalModelManager::new(Arc::clone(&database)).with_engine_dir(engine_dir),
            ),
            database,
            active_runs: Arc::new(Mutex::new(HashMap::new())),
            local_runs: Arc::new(Mutex::new(HashSet::new())),
            local_completions: Arc::new(Mutex::new(HashMap::new())),
            viewed_session: Arc::new(Mutex::new(None)),
            running_subagents: Arc::new(Mutex::new(HashMap::new())),
            // Ask-first, and the setting is read per run so a user who switches
            // to Full does not have to restart the app.
            approval: crate::ai::tools::ApprovalGate::new(crate::ai::tools::PermissionMode::Ask),
        }
    }

    pub fn database(&self) -> Arc<crate::database::Database> {
        Arc::clone(&self.database)
    }

    /// The gate for the current permission mode.
    ///
    /// Returns the runtime's own gate rather than a fresh one, re-pointed at the
    /// stored mode. The gate holds the map of outstanding prompts, so a second
    /// instance would have an empty map while the user's answer went to the first
    /// one: the run would park forever on a prompt nobody could reach, and the
    /// only way out would be cancelling the reply.
    fn approval_gate(
        &self,
        mode: crate::ai::tools::PermissionMode,
        access: crate::ai::tools::AccessSet,
    ) -> crate::ai::tools::ApprovalGate {
        self.approval.with_mode(mode).with_access(access)
    }

    /// Records the user's answer to a pending tool prompt.
    pub fn answer_approval(&self, approval_id: &str, allow: bool) -> bool {
        self.approval.answer(
            approval_id,
            if allow {
                crate::ai::tools::Approval::Allow
            } else {
                crate::ai::tools::Approval::Deny
            },
        )
    }

    /// Records the user's answer to a pending `ask_user` question.
    ///
    /// The reply is free text rather than a decision, which is why it goes to the
    /// gate's question map and not its approval one.
    pub fn answer_question(&self, question_id: &str, answer: String) -> bool {
        self.approval.answer_question(question_id, answer)
    }

    /// Records which session is on screen. `None` means the user is on a new, unsaved
    /// conversation, which matches no run and so never suppresses a notification.
    pub fn set_viewed_session(&self, session_id: Option<String>) {
        if let Ok(mut viewed) = self.viewed_session.lock() {
            *viewed = session_id;
        }
    }

    fn is_viewed(&self, session_id: Option<&str>) -> bool {
        let Some(session_id) = session_id else {
            return false;
        };
        self.viewed_session
            .lock()
            .map(|viewed| viewed.as_deref() == Some(session_id))
            .unwrap_or(false)
    }

    /// The sub-agents currently at work, for a panel that has just opened.
    ///
    /// Read from memory rather than storage because a run is not saved until it
    /// ends; this is the only place that knows about one still going.
    pub fn running_subagents(&self) -> Vec<RunningSubAgent> {
        self.running_subagents
            .lock()
            .map(|running| running.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Tracks sub-agent lifecycle from the event stream the runtime already emits.
    ///
    /// The start and finish of a run are learned from the same `subagent` events
    /// the panel listens for, rather than from a callback threaded through the
    /// tool: every event passes through this one place, so watching here keeps the
    /// registry and the UI reading the same facts instead of two accounts of them.
    fn observe_event(&self, event: &ChatEvent) {
        if event.kind != "subagent" {
            return;
        }
        let Some(metrics) = event.metrics.as_ref() else {
            return;
        };
        let Some(agent_id) = metrics.get("agent_id").and_then(serde_json::Value::as_str) else {
            return;
        };
        let text = |field: &str| {
            metrics
                .get(field)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let Ok(mut running) = self.running_subagents.lock() else {
            return;
        };
        if metrics.get("state").and_then(serde_json::Value::as_str) == Some("running") {
            running.insert(
                agent_id.to_string(),
                RunningSubAgent {
                    id: agent_id.to_string(),
                    label: text("label"),
                    session_id: metrics
                        .get("session_id")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string),
                    prompt: text("prompt"),
                    model: text("model"),
                    provider: text("provider"),
                    started_at: text("started_at"),
                },
            );
        } else {
            // An absent state counts as done, matching the done event's own shape,
            // so an entry can never linger for want of an explicit "done".
            running.remove(agent_id);
        }
    }

    /// Stops a run by setting its cancel flag.
    ///
    /// **A local run's server is left up on purpose.** Stopping a reply and
    /// unloading the model are two different wishes, and killing `llama-server`
    /// to halt one generation made "stop" mean "unload": the user lost a model
    /// they had waited to load and had to pay for it again on the next message.
    /// The loop notices the flag between reads and returns, which drops the
    /// response stream -- the closed stream is what ends the generation.
    pub fn cancel_chat(&self, run_id: &str) {
        if let Some(cancelled) = self.active_runs.lock().unwrap().get(run_id) {
            cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.local_completions.lock().unwrap().remove(run_id);
        self.local_runs.lock().unwrap().remove(run_id);
    }

    fn set_local_completion_id(&self, run_id: &str, completion_id: String) {
        if self.local_runs.lock().unwrap().contains(run_id) {
            self.local_completions
                .lock()
                .unwrap()
                .insert(run_id.to_string(), completion_id);
        }
    }

    pub async fn skip_local_reasoning(&self, run_id: &str) -> Result<(), String> {
        let completion_id = self
            .local_completions
            .lock()
            .unwrap()
            .get(run_id)
            .cloned()
            .ok_or_else(|| {
                "The local model has not started a controllable completion yet".to_string()
            })?;
        self.local_models.end_reasoning(&completion_id).await
    }

    pub fn endpoints(&self) -> &EndpointRegistry {
        &self.endpoints
    }

    /// Ask the configured title models to name a conversation, in order.
    ///
    /// `selections` is the persisted `app.preferences.sessionTitleModels` array:
    /// an ordered list where each entry is a remote `{endpointId, model}` pair
    /// or a `{localModelId}`. `prompt` is the user's opening message only, so
    /// this can run at send time without waiting for the assistant's reply. The
    /// first model that can answer wins, so a local model whose engine is not
    /// running falls through to the next entry instead of failing the request.
    /// If every entry fails, the last error is returned so the reason is still
    /// visible.
    ///
    /// Each model gets up to [`TITLE_ATTEMPTS`] tries. A model that errors, or
    /// answers with something unusable, is retried once before the next entry is
    /// tried; a transient failure or a bad sample should not cost the whole
    /// chain. The cap keeps a systematically unsuitable model from being retried
    /// forever.
    pub async fn generate_title(
        &self,
        selections: &[serde_json::Value],
        prompt: &str,
    ) -> Result<String, String> {
        if selections.is_empty() {
            return Err("No title model selected".to_string());
        }
        let mut last_error = "No title model selected".to_string();
        for selection in selections {
            let (endpoint, model) = match self.resolve_selection(selection) {
                Ok(resolved) => resolved,
                Err(error) => {
                    info!(?selection, %error, "title model unavailable, trying next");
                    last_error = error;
                    continue;
                }
            };
            let messages = [
                serde_json::json!({
                    "role": "system",
                    "content": crate::ai::prompts::title::system_prompt()
                }),
                serde_json::json!({
                    "role": "user",
                    "content": crate::ai::prompts::title::user_prompt(prompt)
                }),
            ];
            for attempt in 1..=TITLE_ATTEMPTS {
                match crate::ai::remote::client::complete_once(
                    &endpoint,
                    &model,
                    &messages,
                    Some(TITLE_MAX_TOKENS),
                )
                .await
                {
                    Ok(raw) => match crate::ai::prompts::title::sanitize_title(&raw) {
                        Some(title) => return Ok(title),
                        None => {
                            // The model answered, but with nothing usable — an
                            // apology, a refusal, or a paragraph. One more try
                            // before giving up on this model.
                            last_error = "The model did not return a usable title".to_string();
                            info!(?selection, attempt, "unusable title output, retrying");
                        }
                    },
                    Err(error) => {
                        info!(?selection, attempt, %error, "title request failed");
                        last_error = error;
                    }
                }
            }
            info!(
                ?selection,
                "title model exhausted its attempts, trying next"
            );
        }
        Err(last_error)
    }

    /// The model a sub-agent should default to, from Settings.
    ///
    /// Read per run and resolved here rather than in the tool, because turning a
    /// stored selection into a live endpoint needs the model registry -- which the
    /// runtime has and a tool does not.
    ///
    /// A selection that no longer resolves is dropped rather than failing the run.
    /// A stale preference -- a provider deleted, an engine not running -- must not
    /// stop the agent doing everything else; the sub-agent simply falls back to
    /// its parent's model.
    fn subagent_default(&self) -> Option<(crate::ai::remote::types::Endpoint, String)> {
        let preferences: serde_json::Value = self.database.setting("app.preferences")?;
        let selection = preferences.get("subagentModel")?;
        if selection.is_null() {
            return None;
        }
        self.resolve_selection(selection).ok()
    }

    /// The model `read_file` shows an image to, from Settings.
    ///
    /// The same shape as [`Self::subagent_default`], for the same reason: the tool
    /// gets a live endpoint rather than a stored selection. `None` when the user
    /// has chosen no vision model, or when the one they chose no longer resolves.
    fn vision_default(&self) -> Option<(crate::ai::remote::types::Endpoint, String)> {
        let preferences: serde_json::Value = self.database.setting("app.preferences")?;
        let selection = preferences.get("visionModel")?;
        if selection.is_null() {
            return None;
        }
        self.resolve_selection(selection).ok()
    }

    /// How aggressively to elide old context, from Settings.
    ///
    /// A plain string preference rather than a resolved model, so it needs none of
    /// the registry work the two above do. An unset or unknown value parses to
    /// `Normal`, which is the safe default: compressing old tool output is never
    /// as bad as a request the provider refuses for being too long.
    fn compaction(&self) -> crate::ai::compact::Compaction {
        let preferences: Option<serde_json::Value> = self.database.setting("app.preferences");
        let stored = preferences
            .as_ref()
            .and_then(|value| value.get("compaction"))
            .and_then(serde_json::Value::as_str);
        crate::ai::compact::Compaction::parse(stored)
    }

    /// Turn one persisted selection into a requestable endpoint and model.
    /// A local selection fails unless its engine is actually running.
    ///
    /// Shared by the two features that store a model choice the same way -- the
    /// title chain and the sub-agent default -- so a local selection is turned
    /// into a live endpoint in exactly one place. Two copies would be two answers
    /// to "which engine is this model on", which is the kind of second opinion
    /// that ends up disagreeing with the loader.
    fn resolve_selection(
        &self,
        selection: &serde_json::Value,
    ) -> Result<(crate::ai::remote::types::Endpoint, String), String> {
        let local_model_id = selection
            .get("localModelId")
            .and_then(serde_json::Value::as_str)
            .filter(|id| !id.trim().is_empty());
        if let Some(local_id) = local_model_id {
            if !self
                .local_models
                .list()
                .iter()
                .any(|model| model.id == local_id)
            {
                return Err("Local model not found".to_string());
            }
            return self
                .local_models
                .loaded_endpoint()
                .ok_or("Local model is not running".to_string());
        }
        let endpoint_id = selection
            .get("endpointId")
            .and_then(serde_json::Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .ok_or("No title model selected")?;
        let endpoint = self
            .endpoints
            .get(endpoint_id)
            .ok_or("The selected title provider no longer exists")?;
        let model = selection
            .get("model")
            .and_then(serde_json::Value::as_str)
            .filter(|model| !model.trim().is_empty())
            .map(str::to_string)
            .or_else(|| endpoint.models.first().cloned())
            .ok_or("The selected title provider has no models")?;
        Ok((endpoint.clone(), model))
    }

    pub fn local_models(&self) -> Arc<LocalModelManager> {
        Arc::clone(&self.local_models)
    }

    /// The saved workspaces and the current selection.
    ///
    /// Read fresh each time rather than cached in the runtime, so a value written
    /// by the settings modal is visible to the header without the two having to
    /// agree on when to refresh.
    pub fn workspaces(&self) -> crate::database::workspaces::Workspaces {
        self.database
            .setting(crate::database::workspaces::WORKSPACES_KEY)
            .unwrap_or_default()
    }

    /// Points every tool at the workspace the user picked.
    ///
    /// **The setting and the process-global are written together here and
    /// nowhere else.** The tools read their root from a global, because a tool
    /// executor is a bare function pointer and cannot capture one, while the list
    /// of open workspaces is durable state. If the two were written separately
    /// they could disagree, and the failure would be silent: the picker showing
    /// one folder while every path resolved against another.
    ///
    /// A selection that cannot be applied -- a folder that has since been
    /// deleted, or one outside the root it was checked against -- leaves the
    /// current root in place and reports why, rather than silently widening what
    /// the tools may read.
    pub fn select_workspace(
        &self,
        path: &str,
    ) -> Result<crate::database::workspaces::Workspaces, String> {
        let mut workspaces = self.workspaces();
        workspaces.select(path)?;
        // Applied before the setting is written, so a rejected folder leaves no
        // trace of having been chosen.
        crate::ai::tools::workspace::set_root(std::path::Path::new(path))?;
        self.database
            .set_setting(crate::database::workspaces::WORKSPACES_KEY, &workspaces)?;
        Ok(workspaces)
    }

    /// Adds a folder to the top of the list and makes it the current root.
    pub fn open_workspace(
        &self,
        path: &str,
    ) -> Result<crate::database::workspaces::Workspaces, String> {
        let mut workspaces = self.workspaces();
        workspaces.add(path);
        crate::ai::tools::workspace::set_root(std::path::Path::new(path))?;
        self.database
            .set_setting(crate::database::workspaces::WORKSPACES_KEY, &workspaces)?;
        Ok(workspaces)
    }

    /// Drops a folder, moving the root if it was the chosen one.
    pub fn close_workspace(
        &self,
        path: &str,
    ) -> Result<crate::database::workspaces::Workspaces, String> {
        let mut workspaces = self.workspaces();
        let was_selected = workspaces.selected.as_deref() == Some(path);
        workspaces.remove(path);
        // Re-applied only when the root actually moves. Canonicalizing an
        // unchanged root would fail outright if that folder had since been
        // deleted, turning the removal of some *other* workspace into an error.
        if was_selected {
            match workspaces.selected.clone() {
                Some(selected) => {
                    crate::ai::tools::workspace::set_root(std::path::Path::new(&selected))?;
                }
                // The last folder was closed, so the tools have nowhere left to
                // work. Cleared rather than left pointing at a folder the user
                // just removed, and rather than defaulted to somewhere else.
                None => crate::ai::tools::workspace::clear_root(),
            }
        }
        self.database
            .set_setting(crate::database::workspaces::WORKSPACES_KEY, &workspaces)?;
        Ok(workspaces)
    }

    /// Restores the stored root at startup.
    ///
    /// Called once from `setup`. A stored selection that no longer resolves --
    /// an external drive that is not mounted, a folder the user deleted -- is
    /// logged and skipped rather than treated as a startup failure, because an
    /// unusable workspace should leave a working app with the tools refusing
    /// rather than no app at all.
    pub fn restore_workspace(&self) {
        let workspaces = self.workspaces();
        let Some(selected) = workspaces.selected.as_deref() else {
            // Nothing chosen, and nothing is assumed. The tools refuse until the
            // user opens a folder, which is the honest outcome.
            return;
        };
        if let Err(error) = crate::ai::tools::workspace::set_root(std::path::Path::new(selected)) {
            info!(%error, %selected, "stored workspace is unusable; no workspace will be active");
        }
    }

    pub async fn run_chat(&self, app: &AppHandle, request: ChatRequest) -> ChatResult {
        let run_id = request.run_id.clone();
        let session_id = request.session_id.clone();
        let session_title = request.session_title.clone();
        let mut seq = 0u64;
        let mut emit = |kind: &str, text: Option<String>| {
            seq += 1;
            let ev = ChatEvent {
                run_id: run_id.clone(),
                session_id: session_id.clone(),
                sequence: seq,
                kind: kind.to_string(),
                text,
                error: None,
                metrics: None,
            };
            let _ = app.emit("ai-event", &ev);
        };

        let is_local = request.local_model;
        let endpoint = if is_local {
            let err = "Load a local model before starting a chat".to_string();
            match self.local_models.loaded_endpoint() {
                Some((endpoint, _)) => endpoint,
                None => {
                    emit("failed", Some(err.clone()));
                    return ChatResult {
                        run_id,
                        content: String::new(),
                        error: Some(err),
                    };
                }
            }
        } else {
            match &request.endpoint_id {
                Some(id) => match self.endpoints.get(id) {
                    Some(ep) => ep,
                    None => {
                        let err = format!("Endpoint '{}' not found", id);
                        emit("failed", Some(err.clone()));
                        return ChatResult {
                            run_id,
                            content: String::new(),
                            error: Some(err),
                        };
                    }
                },
                None => {
                    let err = "No endpoint selected".to_string();
                    emit("failed", Some(err.clone()));
                    return ChatResult {
                        run_id,
                        content: String::new(),
                        error: Some(err),
                    };
                }
            }
        };

        let model = request
            .model
            .clone()
            .unwrap_or_else(|| endpoint.models.first().cloned().unwrap_or_default());

        if model.is_empty() {
            emit("failed", Some("No model selected".to_string()));
            return ChatResult {
                run_id,
                content: String::new(),
                error: Some("No model selected".to_string()),
            };
        }

        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancelled_state = Arc::clone(&cancelled);
        self.active_runs
            .lock()
            .unwrap()
            .insert(run_id.clone(), Arc::clone(&cancelled));
        if is_local {
            self.local_runs.lock().unwrap().insert(run_id.clone());
        }
        emit("status", Some("thinking".to_string()));

        let run_id_clone = run_id.clone();
        let session_id_clone = session_id.clone();
        self.record_skill_uses(&request.skill_ids);
        // Resolved once here rather than inside a tool, because turning the stored
        // selection into a live endpoint needs the model registry, which the tool
        // does not have.
        let subagent_default = self.subagent_default();
        // Resolved here too, for the same reason: the vision choice becomes a live
        // endpoint before it reaches the tool that reads an image.
        let vision_default = self.vision_default();
        // A plain string preference, so it is read here rather than resolved into
        // an endpoint; it rides on the request the way `reasoning` does.
        let compaction = self.compaction();
        // Shareable rather than `FnMut` so several sub-agents launched from this
        // run can each report through the one event path the frontend listens on.
        let emitter = self.clone();
        let app_handle = app.clone();
        let event_run_id = run_id_clone.clone();
        let event_session_id = session_id_clone.clone();
        let on_event: Arc<dyn Fn(ChatEvent) + Send + Sync> = Arc::new(move |ev| {
            if is_local && ev.kind == "completion_id" {
                if let Some(completion_id) = ev.text.clone() {
                    emitter.set_local_completion_id(&event_run_id, completion_id);
                }
            }
            // Tracked before the event is handed out, so the registry and the
            // panel see the same start and the same finish.
            emitter.observe_event(&ev);
            let event = ChatEvent {
                session_id: event_session_id.clone(),
                ..ev
            };
            let _ = app_handle.emit("ai-event", &event);
        });
        let search_app = app.clone();
        let search_run_id = run_id_clone.clone();
        let search_session_id = session_id_clone.clone();
        let on_search: Arc<dyn Fn(crate::websearch::WebSearchOutput) + Send + Sync> = Arc::new(
            move |search| {
                let _ = search_app.emit("ai-event", serde_json::json!({ "run_id": search_run_id, "session_id": search_session_id, "kind": "web_search", "search": search }));
            },
        );
        let result = stream_chat(
            &endpoint,
            &model,
            &request.messages,
            &request.attachments,
            request.reasoning,
            request.web_search_enabled,
            request.mode,
            is_local,
            &run_id_clone,
            session_id.as_deref(),
            Some(&self.database),
            self.approval_gate(request.permission, request.access.unwrap_or_default()),
            cancelled,
            subagent_default,
            vision_default,
            compaction,
            // The top-level run may delegate; a sub-agent's own run passes false.
            true,
            on_event,
            on_search,
        )
        .await;
        self.active_runs.lock().unwrap().remove(&run_id);
        self.local_runs.lock().unwrap().remove(&run_id);
        let result = if cancelled_state.load(std::sync::atomic::Ordering::Relaxed) {
            Err(crate::ai::STOPPED.to_string())
        } else {
            result
        };

        match result {
            Ok((content, metrics)) => {
                let event = ChatEvent {
                    run_id: run_id.clone(),
                    session_id: session_id.clone(),
                    sequence: 0,
                    kind: "completed".to_string(),
                    text: Some(content.clone()),
                    error: None,
                    metrics: Some(metrics),
                };
                let _ = app.emit("ai-event", &event);
                info!(run_id, len = content.len(), "chat completed");
                // Only notify for a reply the user cannot already see. A background run
                // in another session still notifies, even with the window focused.
                if response_notifications_enabled(&self.database)
                    && !self.is_viewed(session_id.as_deref())
                {
                    crate::observation::notification::notify_response_completed(
                        app,
                        session_title.as_deref(),
                    );
                }
                ChatResult {
                    run_id,
                    content,
                    error: None,
                }
            }
            Err(err) => {
                // A stop is the user ending the reply, not a failure. Reported as
                // `completed` with no content so the frontend clears its running
                // state without showing "Request failed: Stopped", and without
                // appending an empty assistant message to the transcript.
                if err == crate::ai::STOPPED {
                    let _ = app.emit(
                        "ai-event",
                        ChatEvent {
                            run_id: run_id.clone(),
                            session_id: session_id.clone(),
                            sequence: 0,
                            kind: "stopped".to_string(),
                            text: Some(String::new()),
                            error: None,
                            metrics: None,
                        },
                    );
                    info!(run_id, "chat stopped");
                    return ChatResult {
                        run_id,
                        content: String::new(),
                        error: None,
                    };
                }
                emit("failed", Some(err.clone()));
                ChatResult {
                    run_id,
                    content: String::new(),
                    error: Some(err),
                }
            }
        }
    }

    /// Counts the skills a run applied.
    ///
    /// Ids only, and deliberately not the instructions: the frontend already built
    /// those into the message list, so resolving them again here would be a second
    /// implementation of the same fact, free to disagree with the one the model was
    /// actually sent.
    ///
    /// **An id that no longer resolves costs nothing.** A skill can be deleted in
    /// Settings while a run is in flight, and the counter is the only thing lost --
    /// the reply keeps the instructions that were already sent.
    fn record_skill_uses(&self, ids: &[String]) {
        for id in ids {
            self.database.record_skill_use(id);
        }
    }
}

fn response_notifications_enabled(database: &std::sync::Arc<crate::database::Database>) -> bool {
    database
        .setting::<serde_json::Value>("app.general")
        .and_then(|settings| {
            settings
                .get("responseNotifications")
                .and_then(serde_json::Value::as_bool)
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runtime backed by a throwaway database, with no local model loaded.
    /// `label` keeps each test on its own file so they can run in parallel.
    fn runtime_without_local_engine(label: &str) -> AiRuntime {
        let database = std::sync::Arc::new(
            crate::database::Database::open(std::env::temp_dir().join(format!(
                "projectz-title-test-{}-{label}.sqlite3",
                std::process::id()
            )))
            .expect("test database"),
        );
        database.initialize().expect("migrate test database");
        AiRuntime::new_with_database(std::env::temp_dir(), database)
    }

    /// A model that cannot be resolved is skipped without consuming an attempt,
    /// so an offline local engine does not cost the chain a retry budget.
    #[tokio::test]
    async fn an_unresolvable_model_does_not_consume_a_retry() {
        let runtime = runtime_without_local_engine("unresolvable");
        let selections = vec![serde_json::json!({"localModelId": "missing-local"})];
        let error = runtime
            .generate_title(&selections, "How do I center a div?")
            .await
            .expect_err("no model is available");
        // The reported reason is the resolution failure, not an exhausted
        // request, which is what a spent attempt would have produced.
        assert_eq!(error, "Local model not found");
    }

    /// Every model failing surfaces the last real error rather than a generic
    /// message, so a provider rejection stays visible in the notification.
    #[tokio::test]
    async fn all_failures_report_the_reason_they_failed() {
        let runtime = runtime_without_local_engine("all-fail");
        let selections = vec![serde_json::json!({"endpointId": "nope", "model": "x"})];
        let error = runtime
            .generate_title(&selections, "How do I center a div?")
            .await
            .expect_err("provider is unknown");
        assert!(!error.is_empty());
    }

    #[tokio::test]
    async fn rejects_an_empty_model_list() {
        let runtime = runtime_without_local_engine("empty");
        let error = runtime
            .generate_title(&[], "How do I center a div?")
            .await
            .expect_err("empty list should fail");
        assert_eq!(error, "No title model selected");
    }

    #[tokio::test]
    async fn skips_an_offline_local_model_and_reports_the_real_reason() {
        let runtime = runtime_without_local_engine("offline-local");
        // No local engine is running, so the first choice cannot be resolved and
        // the failure surfaces instead of silently titling with another model.
        let selections = vec![
            serde_json::json!({"localModelId": "missing-local"}),
            serde_json::json!({"endpointId": "missing-provider", "model": "gpt-4o-mini"}),
        ];
        let error = runtime
            .generate_title(&selections, "How do I center a div?")
            .await
            .expect_err("all choices unavailable");
        assert_eq!(error, "The selected title provider no longer exists");
    }

    #[tokio::test]
    async fn skips_blank_entries_in_the_chain() {
        let runtime = runtime_without_local_engine("blank-entries");
        let selections = vec![
            serde_json::json!({}),
            serde_json::json!({"localModelId": ""}),
            serde_json::json!({"endpointId": "nope"}),
        ];
        let error = runtime
            .generate_title(&selections, "How do I center a div?")
            .await
            .expect_err("all choices unavailable");
        assert_eq!(error, "The selected title provider no longer exists");
    }

    /// The reply the user is watching must stay silent; a run in any other session, or
    /// with no session on screen, is a background run and still notifies.
    #[test]
    fn only_the_session_on_screen_suppresses_its_notification() {
        let runtime = runtime_without_local_engine("viewed-session");
        // Nothing on screen yet: an unsaved new chat matches no stored session, so a
        // finishing reply still has to notify.
        assert!(!runtime.is_viewed(Some("session-1")));
        runtime.set_viewed_session(Some("session-1".to_string()));
        assert!(runtime.is_viewed(Some("session-1")));
        assert!(!runtime.is_viewed(Some("session-2")));
        assert!(!runtime.is_viewed(None));
        // Switching away stops suppressing, so a later background reply notifies again.
        runtime.set_viewed_session(None);
        assert!(!runtime.is_viewed(Some("session-1")));
    }

    /// The running registry follows a sub-agent from start to finish, which is
    /// what lets a panel opened mid-run list an agent already at work.
    #[test]
    fn the_running_registry_follows_a_sub_agent_from_start_to_finish() {
        let runtime = runtime_without_local_engine("running-subagents");
        let lifecycle = |state: &str| crate::ai::types::ChatEvent {
            run_id: "agent-1".into(),
            session_id: None,
            sequence: 0,
            kind: "subagent".into(),
            text: Some("car research".into()),
            error: None,
            metrics: Some(serde_json::json!({
                "agent_id": "agent-1",
                "label": "car research",
                "session_id": "chat-1",
                "state": state,
                "prompt": "Find every car",
                "model": "gpt",
                "provider": "provider-1",
                "started_at": "2026-01-01T00:00:00Z",
            })),
        };

        runtime.observe_event(&lifecycle("running"));
        let running = runtime.running_subagents();
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].id, "agent-1");
        assert_eq!(running[0].label, "car research");
        assert_eq!(running[0].session_id.as_deref(), Some("chat-1"));
        assert_eq!(running[0].prompt, "Find every car");

        // Any other event on the wire is ignored rather than misread as a
        // lifecycle one the registry should act on.
        runtime.observe_event(&crate::ai::types::ChatEvent {
            run_id: "agent-1".into(),
            session_id: None,
            sequence: 1,
            kind: "delta".into(),
            text: Some("hello".into()),
            error: None,
            metrics: None,
        });
        assert_eq!(runtime.running_subagents().len(), 1);

        runtime.observe_event(&lifecycle("done"));
        assert!(runtime.running_subagents().is_empty());
    }
}
