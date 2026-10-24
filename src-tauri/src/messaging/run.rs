//! One message from a chat, answered by the agent.
//!
//! # Order matters, and it is deliberate
//!
//! A button press and a command are handled *before* the per-sender lock is taken.
//! That is not an optimisation: `/stop` and an Allow button exist precisely because
//! a run is already going, and a gate that made them wait for it would be a gate that
//! cannot be used when it is needed. A second *message* does wait -- and is told so
//! rather than queued behind a two-minute run it cannot see.
//!
//! # A message is an ordinary conversation
//!
//! The session, its history and its transcript are the app's own. The run goes
//! through `ai::run::run_prompt`, so tools, sub-agents, compaction and permissions
//! are the ones every other run has, and the conversation opens in the chat view
//! like any other.

use std::path::Path;
use std::sync::Arc;

use serde_json::Value;

use crate::ai::run::{run_prompt, PromptRun};
use crate::ai::types::{ChatEvent, ChatMessage};
use crate::messaging::adapter::Channel;
use crate::messaging::commands::{self, selection_label};
use crate::messaging::session;
use crate::messaging::types::{Button, Inbound};
use crate::messaging::{format, local, relay, ChatState, Messaging};

pub async fn handle(
    hub: Arc<Messaging>,
    channel: Arc<dyn Channel>,
    inbound: Inbound,
) -> Result<(), String> {
    let key = format!("{}:{}", channel.id(), inbound.user_id);
    hub.update_state(&key, |state| state.chat_id = inbound.chat_id.clone());

    let pressed = inbound
        .action()
        .map(|(action_id, data)| (action_id.to_string(), data.to_string()));
    if let Some((action_id, data)) = pressed {
        return action(hub, channel, key, inbound, action_id, data).await;
    }

    let text = inbound.text.trim().to_string();
    if text.is_empty() {
        return Ok(());
    }

    if let Some(intent) = commands::parse(&text) {
        return commands::handle(&hub, channel.as_ref(), &key, &inbound, intent).await;
    }

    // A typed answer to an `ask_user` belongs to the parked run, not to a new one.
    if let Some(question_id) = hub.state(&key).question.clone() {
        hub.update_state(&key, |state| state.question = None);
        if hub.runtime().answer_question(&question_id, text.clone()) {
            return say(channel.as_ref(), &inbound.chat_id, "Answered.").await;
        }
    }

    // One message at a time per sender, so a transcript is never loaded while the
    // turn before it is still being written.
    let lock = hub.chat_lock(&key);
    let Ok(_guard) = lock.try_lock() else {
        return say(
            channel.as_ref(),
            &inbound.chat_id,
            "Still working on the last message. /stop cancels it.",
        )
        .await;
    };

    converse(hub, channel, key, inbound, text).await
}

/// The models to try, in order.
///
/// A model chosen with `/model` is used alone: the user named it, so silently falling
/// back to another one would answer with a model they did not pick. Otherwise the
/// chain from the Model section is used, first to last -- the same shape the title chain has,
/// and the reason there is no model on the Communications page any more.
fn candidates(hub: &Messaging, state: &ChatState) -> Vec<Value> {
    if let Some(chosen) = state.model.clone() {
        return vec![chosen];
    }
    let preferences: Value = hub
        .database()
        .setting("app.preferences")
        .unwrap_or_default();
    preferences
        .get("messagingModels")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// The local model in this selection, when it is not the one loaded.
fn unloaded_local(hub: &Messaging, selection: &Value) -> Option<(String, String)> {
    let id = selection.get("localModelId").and_then(Value::as_str)?;
    if local::is_loaded(hub, id) {
        return None;
    }
    Some((id.to_string(), commands::selection_label(selection)))
}

/// Runs one message and answers it.
async fn converse(
    hub: Arc<Messaging>,
    channel: Arc<dyn Channel>,
    key: String,
    inbound: Inbound,
    text: String,
) -> Result<(), String> {
    let settings = hub.settings();
    let state = hub.state(&key);
    let selections = candidates(&hub, &state);
    let Some(first) = selections.first() else {
        return say(
            channel.as_ref(),
            &inbound.chat_id,
            "No model is set for messaging. Add up to three in ProjectZ → Settings → \
             Model → Main model, or send /model.",
        )
        .await;
    };

    // A local model has to be running before it can answer, and loading one stops
    // whatever the window is using. Ask, rather than replacing it silently.
    if let Some((_, name)) = unloaded_local(&hub, first) {
        if !settings.allow_local_start {
            return say(
                channel.as_ref(),
                &inbound.chat_id,
                &format!(
                    "{name} is not loaded. Load it in ProjectZ, or turn on \
                     \"Let Telegram start a local model\" in Settings → Communications."
                ),
            )
            .await;
        }
        hub.update_state(&key, |state| {
            state.pending = Some(text.clone());
            state.pending_model = Some(first.clone());
        });
        return channel
            .send(
                &inbound.chat_id,
                &format!(
                    "{name} is not loaded.\n\nStarting it unloads the model the app is using. Continue?"
                ),
                &[Button::new("Start it", "ls:1"), Button::new("Not now", "ls:0")],
            )
            .await
            .map(|_| ());
    }

    let database = hub.database();
    // The conversation is resolved **once**, before any model is tried. A fallback
    // that reloaded it would find the failed attempt's own error turn and send that
    // to the next model, which is a conversation arguing with itself.
    let state = hub.state(&key);
    let session_id = match state.session_id.clone() {
        Some(id) if database.session_record(&id).is_ok() => id,
        _ => uuid::Uuid::new_v4().to_string(),
    };
    if state.session_id.as_deref() != Some(session_id.as_str()) {
        hub.bind_session(&key, Some(session_id.clone()));
    }
    let title = session::existing_title(database, &session_id)
        .unwrap_or_else(|| session::title_from(&text));
    let created_at = session::created_at(database, &session_id);
    // Two lists: what the model is sent, and what the store gets back. The second is
    // the record as it was plus this turn, because flattening it would delete fields
    // the model never saw.
    let mut stored = session::stored(database, &session_id);
    let mut messages = session::to_messages(&stored);
    messages.push(ChatMessage {
        role: "user".to_string(),
        content: text.clone(),
    });
    stored.push(serde_json::json!({ "role": "user", "content": text }));

    let total = selections.len();
    let mut last_error: Option<String> = None;
    for (index, selection) in selections.iter().enumerate() {
        let label = commands::selection_label(selection);
        match attempt(
            &hub,
            &channel,
            &key,
            &inbound.chat_id,
            &session_id,
            &title,
            &created_at,
            &messages,
            &stored,
            selection,
            &settings,
        )
        .await
        {
            // A failure that did real work is final: a tool may have written
            // something, so the turn cannot be replayed on another model.
            Ok(outcome) if outcome.error.is_none() || outcome.tool_steps > 0 || outcome.stopped => {
                return respond(channel.as_ref(), &inbound.chat_id, outcome).await;
            }
            Ok(outcome) => last_error = outcome.error,
            Err(reason) => last_error = Some(reason),
        }
        if index + 1 < total {
            let _ = channel
                .send(
                    &inbound.chat_id,
                    &format!("{label} did not answer. Trying the next model…"),
                    &[],
                )
                .await;
        }
    }

    let reason = last_error.unwrap_or_else(|| "No model answered.".to_string());
    say(
        channel.as_ref(),
        &inbound.chat_id,
        &format!("No model answered after retrying {total} model(s). Last error: {reason}"),
    )
    .await
}

/// One model's turn: the request, the run, and the state it leaves behind.
///
/// The conversation is passed in rather than loaded here, so every fallback attempt
/// starts from the same history -- see the note where it is built.
#[allow(clippy::too_many_arguments)]
async fn attempt(
    hub: &Arc<Messaging>,
    channel: &Arc<dyn Channel>,
    key: &str,
    chat: &str,
    session_id: &str,
    title: &str,
    created_at: &str,
    messages: &[ChatMessage],
    stored: &[Value],
    selection: &Value,
    settings: &crate::messaging::settings::MessagingSettings,
) -> Result<crate::ai::run::PromptOutcome, String> {
    let is_local = selection
        .get("localModelId")
        .and_then(Value::as_str)
        .is_some();
    if let Some((_, name)) = unloaded_local(hub, selection) {
        return Err(format!("{name} is not loaded."));
    }
    let (endpoint, model) = hub.runtime().resolve_selection(selection)?;
    let (endpoint_id, local_model, model_name, provider_label) = if is_local {
        (None, true, None, model.clone())
    } else {
        (
            Some(endpoint.id.clone()),
            false,
            Some(model.clone()),
            model.clone(),
        )
    };

    let run_id = uuid::Uuid::new_v4().to_string();
    hub.update_state(key, |state| state.last_run = Some(run_id.clone()));

    // A message may name the folder it works in; `None` leaves the user's open
    // workspace alone. The tools resolve against a process-global root, so it is
    // put in place for the run and restored afterwards -- on every path.
    let previous_root = crate::ai::tools::workspace::root();
    if let Some(path) = settings.workspace.as_deref() {
        if let Err(error) = crate::ai::tools::workspace::set_root(Path::new(path)) {
            hub.update_state(key, |state| state.last_run = None);
            return Err(format!("That workspace cannot be used: {error}"));
        }
    }

    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel::<ChatEvent>();
    let relay_task = tokio::spawn(relay::watch(
        Arc::clone(hub),
        Arc::clone(channel),
        key.to_string(),
        chat.to_string(),
        receiver,
    ));
    let relay_sender = sender.clone();
    let on_event: Arc<dyn Fn(&ChatEvent) + Send + Sync> = Arc::new(move |event| {
        let _ = relay_sender.send(event.clone());
    });

    // One typing hint at the start; the step lines that follow are the progress.
    if channel.capabilities().typing {
        let _ = channel.typing(chat).await;
    }

    let outcome = run_prompt(
        hub.app(),
        hub.runtime(),
        PromptRun {
            run_id,
            session_id: session_id.to_string(),
            title: title.to_string(),
            kind: "telegram".to_string(),
            messages: messages.to_vec(),
            stored_messages: stored.to_vec(),
            endpoint_id,
            local_model,
            model: model_name,
            provider_label,
            mode: settings.mode,
            permission: settings.permission,
            // `jobs` is already off in the stored record; the runtime enforces it
            // again in `ai::run`, so a hand-edited record cannot schedule work.
            access: settings.access,
            web_search: settings.access.web,
            created_at: created_at.to_string(),
            on_event: Some(on_event),
        },
    )
    .await;

    drop(sender);
    let _ = relay_task.await;

    match previous_root {
        Some(root) => {
            let _ = crate::ai::tools::workspace::set_root(&root);
        }
        None => crate::ai::tools::workspace::clear_root(),
    }

    hub.update_state(key, |state| {
        state.last_run = None;
        if let Some(tokens) = outcome
            .usage
            .as_ref()
            .and_then(|usage| usage.get("prompt_tokens"))
            .and_then(Value::as_u64)
        {
            state.prompt_tokens = Some(tokens);
        }
    });
    Ok(outcome)
}

/// Sends what a finished run produced.
async fn respond(
    channel: &dyn Channel,
    chat: &str,
    outcome: crate::ai::run::PromptOutcome,
) -> Result<(), String> {
    if outcome.stopped {
        return say(channel, chat, "Stopped.").await;
    }
    if let Some(error) = outcome.error {
        return say(channel, chat, &format!("Request failed: {error}")).await;
    }
    let parts = format::split_message(&outcome.content, channel.capabilities().max_chars);
    if parts.is_empty() {
        return say(channel, chat, "Done.").await;
    }
    for part in parts {
        channel.send(chat, &part, &[]).await?;
    }
    Ok(())
}

/// A press on one of the app's own buttons.
async fn action(
    hub: Arc<Messaging>,
    channel: Arc<dyn Channel>,
    key: String,
    inbound: Inbound,
    action_id: String,
    data: String,
) -> Result<(), String> {
    let mut acknowledgement = "Done.".to_string();

    if let Some(rest) = data.strip_prefix("ap:") {
        if let Some((approval_id, flag)) = rest.rsplit_once(':') {
            let allow = flag == "y";
            acknowledgement = if hub.runtime().answer_approval(approval_id, allow) {
                if allow {
                    "Allowed.".to_string()
                } else {
                    "Denied.".to_string()
                }
            } else {
                "That prompt is no longer waiting.".to_string()
            };
        }
    } else if data == "x:stop" {
        match hub.state(&key).last_run {
            Some(run_id) => {
                hub.runtime().cancel_chat(&run_id);
                acknowledgement = "Stopping.".to_string();
            }
            None => acknowledgement = "Nothing is running.".to_string(),
        }
    } else if let Some(index) = data.strip_prefix("mc:") {
        if index == "back" {
            let menu = hub.state(&key).menu;
            let buttons = commands::category_buttons(&menu);
            let _ = channel
                .send(&inbound.chat_id, "Choose a category:", &buttons)
                .await;
            acknowledgement = "Choose a category above.".to_string();
        } else if let Some(category) = index
            .parse::<usize>()
            .ok()
            .and_then(|index| hub.state(&key).menu.get(index).cloned())
        {
            hub.update_state(&key, |state| state.choices = category.selections.clone());
            let buttons = commands::model_buttons(&category.selections);
            let _ = channel
                .send(&inbound.chat_id, &format!("{}:", category.label), &buttons)
                .await;
            acknowledgement = "Choose a model above.".to_string();
        }
    } else if let Some(flag) = data.strip_prefix("ls:") {
        if flag != "1" {
            hub.update_state(&key, |state| {
                state.pending = None;
                state.pending_model = None;
            });
            acknowledgement = "Cancelled.".to_string();
        } else {
            let state = hub.state(&key);
            let pending = state.pending;
            let model = state.pending_model;
            hub.update_state(&key, |state| {
                state.pending = None;
                state.pending_model = None;
            });
            let (Some(text), Some(model)) = (pending, model) else {
                channel
                    .answer(&action_id, Some("There is nothing waiting."))
                    .await?;
                return Ok(());
            };
            // Answered before the wait: a load can take minutes, and an
            // unacknowledged press spins for all of it.
            channel
                .answer(&action_id, Some("Starting the model…"))
                .await?;
            let hub = Arc::clone(&hub);
            let channel = Arc::clone(&channel);
            let key = key.clone();
            let mut next = inbound.clone();
            next.text = text.clone();
            tokio::spawn(async move {
                let Some(id) = model.get("localModelId").and_then(Value::as_str) else {
                    let _ = channel
                        .send(&next.chat_id, "That selection is not a local model.", &[])
                        .await;
                    return;
                };
                match local::ensure_loaded(&hub, id).await {
                    Ok(()) => {
                        let _ = channel
                            .send(&next.chat_id, "Loaded. Working on your message.", &[])
                            .await;
                        // The lock is taken here rather than in `handle`, because
                        // this is a continuation of the message that was waiting.
                        let lock = hub.chat_lock(&key);
                        let _guard = lock.lock().await;
                        if let Err(error) =
                            converse(Arc::clone(&hub), Arc::clone(&channel), key, next, text).await
                        {
                            tracing::warn!(%error, "a conversation failed after a model was started");
                        }
                    }
                    Err(reason) => {
                        let _ = channel
                            .send(&next.chat_id, &format!("Could not start it: {reason}"), &[])
                            .await;
                    }
                }
            });
            return Ok(());
        }
    } else if let Some(index) = data.strip_prefix("m:") {
        let choices = hub.state(&key).choices;
        if let Some(choice) = index
            .parse::<usize>()
            .ok()
            .and_then(|index| choices.get(index))
            .cloned()
        {
            hub.update_state(&key, |state| state.model = Some(choice.clone()));
            acknowledgement = format!("Model set to {}.", selection_label(&choice));
        }
    } else if let Some(id) = data.strip_prefix("s:") {
        hub.bind_session(&key, Some(id.to_string()));
        hub.update_state(&key, |state| {
            state.question = None;
            state.choices.clear();
        });
        acknowledgement = "Switched conversation. Send a message to continue it.".to_string();
    } else if data == "n:1" {
        hub.bind_session(&key, None);
        hub.update_state(&key, |state| {
            state.question = None;
            state.choices.clear();
        });
        acknowledgement = "New conversation.".to_string();
    }

    // Always acknowledged, whatever happened: an unacknowledged press spins.
    channel.answer(&action_id, Some(&acknowledgement)).await?;
    // The model and conversation changes are worth a line in the chat itself; the
    // toast that answers the press disappears in a second.
    if data.starts_with("m:") || data.starts_with("s:") || data == "n:1" {
        let _ = channel.send(&inbound.chat_id, &acknowledgement, &[]).await;
    }
    Ok(())
}

async fn say(channel: &dyn Channel, chat: &str, text: &str) -> Result<(), String> {
    channel.send(chat, text, &[]).await.map(|_| ())
}

#[cfg(test)]
mod tests {
    /// A message that is only whitespace is not a message. Telegram can deliver an
    /// empty caption, and starting a run on it would spend a request on nothing.
    #[test]
    fn empty_text_is_not_a_turn() {
        assert!("   \n ".trim().is_empty());
        assert!(!"hello".trim().is_empty());
    }

    /// The prefixes are the contract between a button and the handler; each one is
    /// short enough for Telegram's 64-byte callback limit.
    #[test]
    fn button_data_fits_the_transport_limit() {
        let data = format!("ap:{}:y", "00000000-0000-0000-0000-000000000000");
        assert!(data.len() <= 64);
        let session = format!("s:{}", "00000000-0000-0000-0000-000000000000");
        assert!(session.len() <= 64);
    }
}
