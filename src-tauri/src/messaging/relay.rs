//! What a run looks like from a chat window.
//!
//! # Only two things cross the wire while a run works
//!
//! A finished tool call, and a question that needs the user. Everything else --
//! reasoning, notes, streamed deltas -- is deliberately dropped. A chat window has
//! no hover, no disclosure and no scrollback that a 90-second deliberation deserves;
//! pasting it there would bury the answer that was asked for. The app's own window
//! still draws all of it, because the transcript is saved regardless.
//!
//! # One message that grows, not one per step
//!
//! A run that reads twelve files would otherwise send twelve messages, each of which
//! buzzes a pocket. The working message is edited in place, and edits are throttled:
//! Telegram allows about one edit a second per chat, and a burst of quick tools would
//! spend the allowance on updates nobody can read anyway.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::ai::types::ChatEvent;
use crate::messaging::adapter::Channel;
use crate::messaging::format;
use crate::messaging::types::Button;
use crate::messaging::Messaging;

/// The least time between two edits of the working message.
const EDIT_INTERVAL: Duration = Duration::from_millis(1_200);

/// Watches a run and keeps the chat in step with it.
pub async fn watch(
    hub: Arc<Messaging>,
    channel: Arc<dyn Channel>,
    key: String,
    chat: String,
    mut events: UnboundedReceiver<ChatEvent>,
) {
    let capabilities = channel.capabilities();
    let mut lines: Vec<String> = Vec::new();
    let mut working: Option<String> = None;
    let mut last_edit = Instant::now() - EDIT_INTERVAL;

    while let Some(event) = events.recv().await {
        match event.kind.as_str() {
            // The run has started. A message is posted at once rather than waiting
            // for the first tool: a reply that thinks for a minute would otherwise
            // show only a typing hint that expires after five seconds.
            "status" => {
                if !capabilities.edit || working.is_some() {
                    continue;
                }
                let stop = if capabilities.buttons {
                    vec![Button::new("Stop", "x:stop")]
                } else {
                    Vec::new()
                };
                working = channel
                    .send(&chat, &format::working_text(&lines), &stop)
                    .await
                    .ok()
                    .map(|sent| sent.id);
                last_edit = Instant::now();
                continue;
            }
            "tool_result" => {
                if let Some(line) = format::tool_line(&event) {
                    lines.push(line);
                }
            }
            "tool_approval" => {
                post_approval(hub.as_ref(), channel.as_ref(), &chat, &event).await;
                continue;
            }
            "user_question" => {
                post_question(hub.as_ref(), channel.as_ref(), &key, &chat, &event).await;
                continue;
            }
            "usage" => {
                if let Some(tokens) = event
                    .metrics
                    .as_ref()
                    .and_then(|metrics| metrics.get("prompt_tokens"))
                    .and_then(Value::as_u64)
                {
                    hub.update_state(&key, |state| state.prompt_tokens = Some(tokens));
                }
                continue;
            }
            // Deliberately nothing: a thought, a note and a streamed word are the
            // app's own view, not this one's.
            _ => continue,
        }

        if !capabilities.edit || last_edit.elapsed() < EDIT_INTERVAL {
            continue;
        }
        let text = format::working_text(&lines);
        match working.clone() {
            None => {
                let stop = if capabilities.buttons {
                    vec![Button::new("Stop", "x:stop")]
                } else {
                    Vec::new()
                };
                working = channel
                    .send(&chat, &text, &stop)
                    .await
                    .ok()
                    .map(|sent| sent.id);
            }
            Some(id) => {
                let _ = channel.edit(&chat, &id, &text, &[]).await;
            }
        }
        last_edit = Instant::now();
    }

    // The answer is sent by the caller; this leaves behind what produced it, so the
    // answer is not floating under a message that still says "Working…".
    if let Some(id) = working {
        let _ = channel
            .edit(&chat, &id, &format::worked_summary(&lines), &[])
            .await;
    }
}

/// Relays a pending tool prompt as a pair of buttons.
///
/// The buttons carry the approval id because that is the only thing the runtime
/// needs to settle the call, and it is what makes a prompt answerable from a phone
/// at all -- without this an unattended run at `auto_writes` would park forever on
/// the first terminal command.
async fn post_approval(hub: &Messaging, channel: &dyn Channel, chat: &str, event: &ChatEvent) {
    let Some(metrics) = event.metrics.as_ref() else {
        return;
    };
    let Some(approval_id) = metrics.get("approval_id").and_then(Value::as_str) else {
        return;
    };
    let summary = event
        .text
        .clone()
        .unwrap_or_else(|| "A tool wants to run.".to_string());
    let mut buttons = Vec::new();
    if channel.capabilities().buttons {
        buttons.push(Button::new("Allow", format!("ap:{approval_id}:y")));
        buttons.push(Button::new("Deny", format!("ap:{approval_id}:n")));
    }
    let text = if buttons.is_empty() {
        format!("{summary}\n\nReply /allow {approval_id} or /deny {approval_id}.")
    } else {
        format!("{summary}\n\nAllow this?")
    };
    let _ = channel.send(chat, &text, &buttons).await;
    let _ = hub;
}

/// Relays `ask_user`, and remembers which question a typed answer belongs to.
async fn post_question(
    hub: &Messaging,
    channel: &dyn Channel,
    key: &str,
    chat: &str,
    event: &ChatEvent,
) {
    let Some(question_id) = event
        .metrics
        .as_ref()
        .and_then(|metrics| metrics.get("question_id"))
        .and_then(Value::as_str)
    else {
        return;
    };
    hub.update_state(key, |state| state.question = Some(question_id.to_string()));
    let text = event
        .text
        .clone()
        .unwrap_or_else(|| "The agent has a question.".to_string());
    let _ = channel
        .send(chat, &format!("{text}\n\nReply with your answer."), &[])
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The throttle is short enough that a slow tool still shows, and long enough
    /// that a burst of fast ones does not spend Telegram's edit allowance.
    #[test]
    fn the_edit_interval_is_a_second_and_a_bit() {
        assert!(EDIT_INTERVAL >= Duration::from_millis(1_000));
        assert!(EDIT_INTERVAL <= Duration::from_secs(2));
    }
}
