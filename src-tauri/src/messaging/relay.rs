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

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::ai::types::ChatEvent;
use crate::messaging::adapter::Channel;
use crate::messaging::files;
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
    agent: bool,
    send_files: bool,
) {
    let capabilities = channel.capabilities();
    let mut lines: Vec<String> = Vec::new();
    let mut changed: BTreeSet<String> = BTreeSet::new();
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
                if let Some(path) = files::changed_file(&event) {
                    changed.insert(path);
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

    // The answer is sent by the caller. An agent that actually did steps leaves
    // the one-line record of them; a single-message Chat reply (or an Agent reply
    // that used no tools) has no work worth keeping, so its working message is
    // removed rather than left saying "Worked for 0s".
    if let Some(id) = working {
        if agent && !lines.is_empty() {
            let _ = channel
                .edit(&chat, &id, &format::worked_summary(&lines), &[])
                .await;
        } else if capabilities.delete {
            let _ = channel.delete(&chat, &id).await;
        } else if capabilities.edit {
            let _ = channel.edit(&chat, &id, "", &[]).await;
        }
    }

    // Sent after the working message is settled and before the answer, so files
    // appear above the text that talks about them.
    if send_files {
        files::send_changed(channel.as_ref(), &chat, changed).await;
    }

    // A question that outlived its run is not waiting for anything. Left in the
    // state, the next message from this person would be taken as its answer and
    // replied to with "Answered." -- to a prompt whose run had already ended.
    hub.update_state(&key, |state| {
        state.question = None;
        state.question_options.clear();
    });
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
///
/// The suggested answers go as buttons, because they are the one part of a
/// question a phone can answer without typing. The callback data cannot carry the
/// text -- Telegram caps it at 64 bytes and the question id spends most of that --
/// so the option list is kept in the sender's state and the button carries its
/// index instead. A question with no options is still a typed answer, and says so.
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
    let options: Vec<String> = event
        .metrics
        .as_ref()
        .and_then(|metrics| metrics.get("options"))
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|option| !option.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    hub.update_state(key, |state| {
        state.question = Some(question_id.to_string());
        state.question_options = options.clone();
    });

    let text = event
        .text
        .clone()
        .unwrap_or_else(|| "The agent has a question.".to_string());

    // Four is what fits a phone's keyboard in one screenful without the buttons
    // becoming a list of their own. A fifth suggestion is still in the message.
    let mut buttons = Vec::new();
    if channel.capabilities().buttons && !options.is_empty() {
        for (index, option) in options.iter().enumerate().take(4) {
            buttons.push(Button::new(
                button_label(option),
                format!("qa:{question_id}:{index}"),
            ));
        }
    }
    let text = if !buttons.is_empty() {
        format!("{text}\n\nChoose one, or reply with your own answer.")
    } else {
        let listed = if options.is_empty() {
            String::new()
        } else {
            let lines = options
                .iter()
                .enumerate()
                .map(|(index, option)| format!("{}. {option}", index + 1))
                .collect::<Vec<_>>()
                .join("\n");
            format!("\n\n{lines}")
        };
        format!("{text}{listed}\n\nReply with your answer.")
    };
    let _ = channel.send(chat, &text, &buttons).await;
}

/// A button's caption, shortened to what a phone can show.
///
/// Truncated on characters rather than bytes so an option in a non-Latin script
/// cannot be cut in the middle of a glyph.
fn button_label(text: &str) -> String {
    const MAX_CHARS: usize = 32;
    let trimmed = text.trim();
    if trimmed.chars().count() <= MAX_CHARS {
        return trimmed.to_string();
    }
    let mut label: String = trimmed.chars().take(MAX_CHARS - 1).collect();
    label.push('…');
    label
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

    /// An option is a sentence as often as it is a word, and a button is not the
    /// place for a paragraph. The cut is on characters so a non-Latin option is
    /// not severed mid-glyph.
    #[test]
    fn a_long_option_is_shortened_for_a_button() {
        assert_eq!(button_label("Yes"), "Yes");
        let long = "a".repeat(50);
        let label = button_label(&long);
        assert_eq!(label.chars().count(), 32);
        assert!(label.ends_with('…'));
    }
}
