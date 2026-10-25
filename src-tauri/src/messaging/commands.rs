//! The slash commands, written once for every channel.
//!
//! A command is not a transport feature: `/new` means the same thing on Telegram
//! as it would on Discord, and `/model` has to list the same models either way. The
//! only channel-specific thing is how a choice is drawn, which is
//! [`Capabilities::buttons`]'s question and is asked rather than assumed.
//!
//! # Why the commands are few
//!
//! Every command is a door into the app that bypasses the composer, and a door that
//! only repeats a setting is a second place for the two to disagree. These are the
//! things a phone cannot reach any other way: start over, continue an old
//! conversation, change the model, see how full the context is, stop.

use std::sync::Arc;

use serde_json::Value;

use crate::messaging::adapter::Channel;
use crate::messaging::types::{Button, Inbound};
use crate::messaging::Messaging;

/// How many conversations `/sessions` offers.
///
/// A menu, not a history browser: six is a screenful on a phone, and anything older
/// is easier to find in the app than by pressing next twice.
const SESSIONS_OFFERED: usize = 6;

pub enum Intent {
    Start,
    Help,
    New,
    Stop,
    Model,
    Context,
    Compress,
    Unload,
    Sessions,
    Profile(String),
    Allow(String),
    Deny(String),
}

/// Reads a command, or decides the text is not one.
///
/// Unknown commands are `None` rather than help: `/etc/hosts` is a path somebody
/// pasted, not a typo, and answering it with a menu is worse than sending it to the
/// agent. `@botname` suffixes are stripped, because Telegram puts them on a command
/// typed in a group.
pub fn parse(text: &str) -> Option<Intent> {
    let rest = text.trim().strip_prefix('/')?;
    let (name, argument) = match rest.split_once(char::is_whitespace) {
        Some((name, argument)) => (name, argument.trim()),
        None => (rest, ""),
    };
    let name = name.split('@').next().unwrap_or(name).to_lowercase();
    Some(match name.as_str() {
        "start" => Intent::Start,
        "help" => Intent::Help,
        "new" => Intent::New,
        "stop" => Intent::Stop,
        "model" | "models" => Intent::Model,
        "context" | "ctx" => Intent::Context,
        "compress" | "compact" => Intent::Compress,
        "unload" => Intent::Unload,
        "sessions" | "session" => Intent::Sessions,
        "profile" => Intent::Profile(argument.to_string()),
        "allow" => Intent::Allow(argument.to_string()),
        "deny" => Intent::Deny(argument.to_string()),
        _ => return None,
    })
}

pub async fn handle(
    hub: &Arc<Messaging>,
    channel: &dyn Channel,
    key: &str,
    inbound: &Inbound,
    intent: Intent,
) -> Result<(), String> {
    let say = |text: String| {
        let chat = inbound.chat_id.clone();
        async move { channel.send(&chat, &text, &[]).await.map(|_| ()) }
    };

    match intent {
        Intent::Start => {
            let name = inbound.user_name.trim();
            let greeting = if name.is_empty() {
                help_text()
            } else {
                format!("Hello, {name}.\n\n{}", help_text())
            };
            say(greeting).await
        }
        Intent::Help => say(help_text()).await,
        Intent::New => {
            new_conversation(hub, channel, key, &inbound.chat_id, &inbound.user_id).await
        }
        Intent::Stop => {
            let running = hub.state(key).last_run;
            match running {
                Some(run_id) => {
                    hub.runtime().cancel_chat(&run_id);
                    say("Stopping.".to_string()).await
                }
                None => say("Nothing is running.".to_string()).await,
            }
        }
        Intent::Context => {
            let state = hub.state(key);
            let Some(tokens) = state.prompt_tokens else {
                return say(
                    "No reply has reported a context size yet. Send a message first.".to_string(),
                )
                .await;
            };
            let window = window_for(hub, state.model.as_ref());
            let text = match window {
                Some(window) if window > 0 => {
                    let percent = (tokens as f64 / window as f64 * 100.0).round() as u64;
                    format!(
                        "Context: {} / {} tokens ({percent}%).",
                        thousands(tokens),
                        thousands(window)
                    )
                }
                _ => format!(
                    "Context: {} tokens. This model does not report a window size.",
                    thousands(tokens)
                ),
            };
            say(text).await
        }
        Intent::Model => {
            let categories = crate::messaging::menu::categories(hub);
            if categories.is_empty() {
                return say(
                    "No models are set up. Add a provider or a local model in Settings first."
                        .to_string(),
                )
                .await;
            }
            let buttons = category_buttons(&categories);
            hub.update_state(key, |state| {
                state.menu = categories;
                state.choices.clear();
            });
            let chat = inbound.chat_id.clone();
            channel
                .send(&chat, "Choose a category:", &buttons)
                .await
                .map(|_| ())
        }
        Intent::Compress => {
            let Some(session_id) = hub.state(key).session_id else {
                return say("There is no conversation to trim yet.".to_string()).await;
            };
            let settings = crate::ai::compact::stored(hub.database());
            match crate::ai::compact::compact_session(hub.database(), &session_id, settings) {
                Ok(Some(messages)) => {
                    say(format!("Trimmed. {} messages remain.", messages.len())).await
                }
                Ok(None) => say("Nothing old enough to trim yet.".to_string()).await,
                Err(reason) => say(reason).await,
            }
        }
        Intent::Unload => {
            crate::messaging::local_menu::show(hub, channel, &inbound.chat_id, &inbound.user_id)
                .await
        }
        Intent::Sessions => {
            let headers = hub
                .database()
                .list_session_headers()
                .unwrap_or_default()
                .into_iter()
                .filter(|header| {
                    matches!(
                        header.get("kind").and_then(Value::as_str).unwrap_or("chat"),
                        "chat" | "telegram"
                    )
                })
                .take(SESSIONS_OFFERED)
                .collect::<Vec<_>>();
            if headers.is_empty() {
                return say("No conversations yet. Send a message to start one.".to_string()).await;
            }
            let current = hub.state(key).session_id;
            let mut lines = vec!["Continue a conversation:".to_string()];
            let mut buttons = Vec::new();
            for (index, header) in headers.iter().enumerate() {
                let id = header.get("id").and_then(Value::as_str).unwrap_or_default();
                let title = header
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("Untitled");
                let updated = header
                    .get("updatedAt")
                    .and_then(Value::as_str)
                    .map(|stamp| stamp.get(..10).unwrap_or(stamp))
                    .unwrap_or_default();
                let marker = if current.as_deref() == Some(id) {
                    " ←"
                } else {
                    ""
                };
                lines.push(format!("{}. {title} ({updated}){marker}", index + 1));
                buttons.push(Button::new(format!("{}", index + 1), format!("s:{id}")));
            }
            buttons.push(Button::new("New conversation", "n:new"));
            let chat = inbound.chat_id.clone();
            channel
                .send(&chat, &lines.join("\n"), &buttons)
                .await
                .map(|_| ())
        }
        Intent::Profile(text) => profile(hub, channel, inbound, &text).await,
        Intent::Allow(id) => decide(hub, channel, inbound, &id, true).await,
        Intent::Deny(id) => decide(hub, channel, inbound, &id, false).await,
    }
}

/// Starts the `/new` flow, offering the modes this sender is allowed.
///
/// The old binding is cleared before the question is drawn, so the next message
/// really is the new conversation. The model is kept on purpose: `/new` is about
/// the conversation, not about the configuration.
pub async fn new_conversation(
    hub: &Arc<Messaging>,
    channel: &dyn Channel,
    key: &str,
    chat: &str,
    user_id: &str,
) -> Result<(), String> {
    hub.bind_session(key, None);
    let settings = hub.settings();
    hub.update_state(key, |state| {
        state.question = None;
        state.choices.clear();
        state.mode = None;
        // A Start/Not now prompt belongs to the conversation being left behind.
        state.pending = None;
        state.pending_model = None;
    });

    if !settings.user(user_id).allow_agent {
        hub.update_state(key, |state| {
            state.mode = Some(crate::ai::tools::ToolMode::Chat)
        });
        return channel
            .send(
                chat,
                "New chat conversation. Agent is turned off for your Telegram id in Communications → Security.",
                &[],
            )
            .await
            .map(|_| ());
    }
    if !channel.capabilities().buttons {
        return channel
            .send(chat, "New conversation. Send a message to begin.", &[])
            .await
            .map(|_| ());
    }
    channel
        .send(
            chat,
            "New conversation. Should this one be Chat or Agent?",
            &[
                Button::new("Chat", "n:chat"),
                Button::new("Agent", "n:agent"),
            ],
        )
        .await
        .map(|_| ())
}

/// Reads or writes the sender's own answer profile.
async fn profile(
    hub: &Arc<Messaging>,
    channel: &dyn Channel,
    inbound: &Inbound,
    argument: &str,
) -> Result<(), String> {
    let text = argument.trim();
    let base = crate::messaging::storage::base(hub.as_ref())?;
    if text.is_empty() {
        return crate::messaging::profile_menu::show(
            hub,
            channel,
            &inbound.chat_id,
            &inbound.user_id,
        )
        .await;
    }
    if text.eq_ignore_ascii_case("clear") {
        crate::messaging::profiles::clear(&base, &inbound.user_id).await?;
        return channel
            .send(&inbound.chat_id, "Profile cleared.", &[])
            .await
            .map(|_| ());
    }
    let stored = crate::messaging::profiles::set(&base, &inbound.user_id, text).await?;
    let path = crate::messaging::storage::profile_path(&base, &inbound.user_id)?;
    channel
        .send(
            &inbound.chat_id,
            &format!(
                "Profile saved to {}. It now applies to your messages here:\n\n{stored}",
                path.display()
            ),
            &[],
        )
        .await
        .map(|_| ())
}

/// Answers an approval typed as a command, for a channel or a moment where a
/// button was missed.
async fn decide(
    hub: &Arc<Messaging>,
    channel: &dyn Channel,
    inbound: &Inbound,
    approval_id: &str,
    allow: bool,
) -> Result<(), String> {
    if approval_id.trim().is_empty() {
        return channel
            .send(
                &inbound.chat_id,
                "Use /allow <id> or /deny <id>, with the id from the prompt.",
                &[],
            )
            .await
            .map(|_| ());
    }
    let answered = hub.runtime().answer_approval(approval_id.trim(), allow);
    let text = if answered {
        if allow {
            "Allowed."
        } else {
            "Denied."
        }
    } else {
        "That prompt is no longer waiting."
    };
    channel.send(&inbound.chat_id, text, &[]).await.map(|_| ())
}

fn help_text() -> String {
    [
        "ProjectZ",
        "",
        "/new — start a conversation, as Chat or Agent",
        "/sessions — continue an earlier one",
        "/profile — permissions, tools and answer style",
        "/model — change the model",
        "/context — how full the context is",
        "/compress — trim older tool output now",
        "/unload — choose a running local engine to stop",
        "/stop — stop the current reply",
        "/help — this list",
        "",
        "Anything else is sent to the agent.",
    ]
    .join("\n")
}

/// One button per category, with the count so a reader knows the depth first.
pub fn category_buttons(categories: &[crate::messaging::menu::Category]) -> Vec<Button> {
    categories
        .iter()
        .enumerate()
        .map(|(index, category)| Button::new(category.label.clone(), format!("mc:{index}")))
        .collect()
}

/// The models of one category, plus a way back to the list.
pub fn model_buttons(selections: &[Value]) -> Vec<Button> {
    let mut buttons: Vec<Button> = selections
        .iter()
        .enumerate()
        .map(|(index, selection)| Button::new(model_button_label(selection), format!("m:{index}")))
        .collect();
    buttons.push(Button::new("Back", "mc:back"));
    buttons
}

/// How a selection is named in a message.
pub fn selection_label(selection: &Value) -> String {
    if let Some(id) = selection.get("localModelId").and_then(Value::as_str) {
        return selection
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(id)
            .to_string();
    }
    selection
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("unknown model")
        .to_string()
}

/// A button label, short enough to fit a phone's keyboard.
pub fn model_button_label(selection: &Value) -> String {
    let label = selection_label(selection);
    if selection.get("localModelId").is_some() {
        return format!("{label} (local)");
    }
    label.chars().take(40).collect()
}

fn window_for(hub: &Arc<Messaging>, selection: Option<&Value>) -> Option<u64> {
    let selection = selection?;
    if selection.get("localModelId").is_some() {
        return hub.runtime().local_models().context_window();
    }
    let (endpoint, model) = hub.runtime().resolve_selection(selection).ok()?;
    hub.runtime()
        .database()
        .model_capabilities(&endpoint.id, &model)
        .ok()
        .and_then(|capabilities| capabilities.context_length)
        .and_then(|context| u64::try_from(context).ok())
}

/// Thousands separators, because `12400` is read wrong at a glance.
pub fn thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(character);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_commands_are_recognised() {
        assert!(matches!(parse("/new"), Some(Intent::New)));
        assert!(matches!(parse("/stop"), Some(Intent::Stop)));
        assert!(matches!(parse("/context"), Some(Intent::Context)));
        assert!(matches!(parse("/unload"), Some(Intent::Unload)));
        assert!(matches!(parse("/sessions"), Some(Intent::Sessions)));
        assert!(matches!(parse("/model"), Some(Intent::Model)));
        assert!(matches!(parse("/help"), Some(Intent::Help)));
        assert!(matches!(parse("/start"), Some(Intent::Start)));
        assert!(matches!(
            parse("/profile short answers"),
            Some(Intent::Profile(_))
        ));
    }

    /// A command typed in a group arrives as `/new@projectz_bot`; the suffix is not
    /// part of the name.
    #[test]
    fn a_bot_suffix_is_stripped() {
        assert!(matches!(parse("/new@projectz_bot"), Some(Intent::New)));
    }

    /// An approval's id is the argument, because that is the only thing the model
    /// cannot supply and the prompt does not print.
    #[test]
    fn allow_takes_the_id() {
        match parse("/allow abc-123") {
            Some(Intent::Allow(id)) => assert_eq!(id, "abc-123"),
            _ => panic!("expected allow"),
        }
    }

    /// **A path is not a command.** Answering a pasted `ls` with a menu would be
    /// worse than sending it to the agent, which is what will happen.
    #[test]
    fn an_unknown_slash_is_text() {
        assert!(parse("/etc/hosts").is_none());
        assert!(parse("hello").is_none());
    }

    /// Numbers are grouped for reading.
    #[test]
    fn thousands_are_grouped() {
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(12_400), "12,400");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    /// A model is named by the part a person recognises, and a local one says so.
    #[test]
    fn a_selection_is_named_plainly() {
        let remote = json!({ "endpointId": "e", "model": "gpt-4o-mini" });
        assert_eq!(selection_label(&remote), "gpt-4o-mini");
        assert_eq!(model_button_label(&remote), "gpt-4o-mini");
        let local = json!({ "localModelId": "abc", "name": "Qwen 2B" });
        assert_eq!(selection_label(&local), "Qwen 2B");
        assert_eq!(model_button_label(&local), "Qwen 2B (local)");
    }
}
