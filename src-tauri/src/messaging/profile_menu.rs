//! The `/profile` menu: this sender's answer style, permissions and tools.
//!
//! A Telegram user can change their own policy inside the installation ceiling
//! without opening the desktop app. The style text itself still uses
//! `/profile <text>`; the menu covers the switches that are awkward to type.

use std::sync::Arc;

use crate::ai::tools::{AccessSet, PermissionMode};
use crate::messaging::adapter::Channel;
use crate::messaging::storage;
use crate::messaging::types::Button;
use crate::messaging::Messaging;

const TOOLS: [(&str, &str); 7] = [
    ("read", "Read"),
    ("write", "Write"),
    ("terminal", "Terminal"),
    ("web", "Web"),
    ("subagents", "Subagents"),
    ("jobs", "Jobs"),
    ("mcp", "MCP"),
];

fn permission_name(permission: PermissionMode) -> &'static str {
    match permission {
        PermissionMode::Ask => "Ask each time",
        PermissionMode::AutoSafe => "Auto reads",
        PermissionMode::AutoWrites => "Auto writes",
        PermissionMode::Full => "Full access",
    }
}

fn permission_from(value: &str) -> Option<PermissionMode> {
    match value {
        "ask" => Some(PermissionMode::Ask),
        "auto_safe" => Some(PermissionMode::AutoSafe),
        "auto_writes" => Some(PermissionMode::AutoWrites),
        "full" => Some(PermissionMode::Full),
        _ => None,
    }
}

fn access_value(access: &AccessSet, tool: &str) -> bool {
    match tool {
        "read" => access.read,
        "write" => access.write,
        "terminal" => access.terminal,
        "web" => access.web,
        "subagents" => access.subagents,
        "jobs" => access.jobs,
        "mcp" => access.mcp,
        _ => false,
    }
}

fn toggle_access(access: &mut AccessSet, tool: &str) {
    match tool {
        "read" => access.read = !access.read,
        "write" => access.write = !access.write,
        "terminal" => access.terminal = !access.terminal,
        "web" => access.web = !access.web,
        "subagents" => access.subagents = !access.subagents,
        "jobs" => access.jobs = !access.jobs,
        "mcp" => access.mcp = !access.mcp,
        _ => {}
    }
}

/// What `/profile` shows, with the permission choices in one press.
pub async fn show(
    hub: &Arc<Messaging>,
    channel: &dyn Channel,
    chat: &str,
    user_id: &str,
) -> Result<(), String> {
    let user = hub.settings().user(user_id);
    let base = storage::base(hub.as_ref())?;
    let style = crate::messaging::profiles::get(&base, user_id)
        .await
        .map(|text| {
            let first = text.lines().next().unwrap_or_default().trim();
            if first.chars().count() > 90 {
                format!("{}…", first.chars().take(90).collect::<String>())
            } else {
                first.to_string()
            }
        })
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "not set".to_string());
    let tools = TOOLS
        .iter()
        .filter(|(key, _)| access_value(&user.access, key))
        .map(|(_, label)| *label)
        .collect::<Vec<_>>()
        .join(", ");

    let text = format!(
        "Your Telegram profile\n\nStyle: {style}\nPermission: {}\nTools: {}\nStart local chat model: {}\nStart speech-to-text: {}\n\nChoose a permission, or open a menu below.",
        permission_name(user.permission),
        if tools.is_empty() { "none".to_string() } else { tools },
        if user.allow_local_start { "on" } else { "off" },
        if user.allow_stt_start { "on" } else { "off" },
    );
    let buttons = vec![
        Button::new("Ask each", "u:p:ask"),
        Button::new("Auto reads", "u:p:auto_safe"),
        Button::new("Auto writes", "u:p:auto_writes"),
        Button::new("Full access", "u:p:full"),
        Button::new("Tools", "u:t"),
        Button::new("Local starts", "u:l"),
        Button::new("Style help", "u:style"),
    ];
    channel.send(chat, &text, &buttons).await.map(|_| ())
}

/// Applies one `/profile` callback. The caller answers the press.
pub async fn handle(
    hub: &Arc<Messaging>,
    channel: &dyn Channel,
    chat: &str,
    user_id: &str,
    data: &str,
) -> Result<String, String> {
    if let Some(level) = data.strip_prefix("u:p:") {
        let permission =
            permission_from(level).ok_or_else(|| "Unknown permission level.".to_string())?;
        update(hub, user_id, |user| user.permission = permission)?;
        show(hub, channel, chat, user_id).await?;
        return Ok(format!(
            "Permission set to {}.",
            permission_name(permission)
        ));
    }

    if data == "u:t" {
        show_tools(hub, channel, chat, user_id).await?;
        return Ok("Choose a tool to toggle.".to_string());
    }
    if let Some(tool) = data.strip_prefix("u:t:") {
        if !TOOLS.iter().any(|(key, _)| *key == tool) {
            return Err("Unknown tool.".to_string());
        }
        update(hub, user_id, |user| toggle_access(&mut user.access, tool))?;
        show_tools(hub, channel, chat, user_id).await?;
        let label = TOOLS
            .iter()
            .find(|(key, _)| *key == tool)
            .map(|(_, label)| *label)
            .unwrap_or(tool);
        return Ok(format!("{label} toggled."));
    }
    if data == "u:l" {
        show_local(hub, channel, chat, user_id).await?;
        return Ok("Choose a local start setting.".to_string());
    }
    if data == "u:l:chat" {
        update(hub, user_id, |user| {
            user.allow_local_start = !user.allow_local_start
        })?;
        show_local(hub, channel, chat, user_id).await?;
        return Ok("Local chat-model start toggled.".to_string());
    }
    if data == "u:l:stt" {
        update(hub, user_id, |user| {
            user.allow_stt_start = !user.allow_stt_start
        })?;
        show_local(hub, channel, chat, user_id).await?;
        return Ok("Speech-to-text start toggled.".to_string());
    }
    if data == "u:style" {
        channel
            .send(
                chat,
                "Answer style is text, not a switch. Send /profile <instructions> to replace it, or /profile clear to remove it. The plain-text channel rules still apply.",
                &[Button::new("Back", "u:main")],
            )
            .await?;
        return Ok("Style help sent.".to_string());
    }

    show(hub, channel, chat, user_id).await?;
    Ok("Profile menu.".to_string())
}

async fn show_tools(
    hub: &Arc<Messaging>,
    channel: &dyn Channel,
    chat: &str,
    user_id: &str,
) -> Result<(), String> {
    let user = hub.settings().user(user_id);
    let mut buttons = TOOLS
        .iter()
        .map(|(key, label)| {
            let state = if access_value(&user.access, key) {
                "on"
            } else {
                "off"
            };
            Button::new(format!("{label}: {state}"), format!("u:t:{key}"))
        })
        .collect::<Vec<_>>();
    buttons.push(Button::new("Back", "u:main"));
    channel
        .send(
            chat,
            "Tools for your Telegram id. The installation limit can still remove one.",
            &buttons,
        )
        .await
        .map(|_| ())
}

async fn show_local(
    hub: &Arc<Messaging>,
    channel: &dyn Channel,
    chat: &str,
    user_id: &str,
) -> Result<(), String> {
    let user = hub.settings().user(user_id);
    channel
        .send(
            chat,
            "Local starts for your Telegram id. Starting one can replace the model the host is using.",
            &[
                Button::new(
                    format!("Chat model: {}", if user.allow_local_start { "on" } else { "off" }),
                    "u:l:chat",
                ),
                Button::new(
                    format!(
                        "Speech-to-text: {}",
                        if user.allow_stt_start { "on" } else { "off" }
                    ),
                    "u:l:stt",
                ),
                Button::new("Back", "u:main"),
            ],
        )
        .await
        .map(|_| ())
}

fn update(
    hub: &Arc<Messaging>,
    user_id: &str,
    apply: impl FnOnce(&mut crate::messaging::settings::MessagingUser),
) -> Result<(), String> {
    let mut settings = hub.settings();
    if !settings.allows(user_id) {
        return Err("This Telegram id is not allowed.".to_string());
    }
    let mut user = settings.user(user_id);
    apply(&mut user);
    // The Agent ceiling still owns the mode field.
    if !user.allow_agent {
        user.mode = crate::ai::tools::ToolMode::Chat;
    }
    settings.users.insert(user_id.trim().to_string(), user);
    hub.save(settings)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_permission_has_a_callback_and_back() {
        for (value, expected) in [
            ("ask", PermissionMode::Ask),
            ("auto_safe", PermissionMode::AutoSafe),
            ("auto_writes", PermissionMode::AutoWrites),
            ("full", PermissionMode::Full),
        ] {
            assert_eq!(permission_from(value), Some(expected));
        }
        assert_eq!(permission_from("everything"), None);
    }

    #[test]
    fn the_profile_tools_match_the_run_access_set() {
        let mut access = AccessSet::ALL;
        toggle_access(&mut access, "terminal");
        assert!(!access.terminal);
        assert!(access.read && access.mcp);
        assert!(access_value(&access, "read"));
        assert!(!access_value(&access, "terminal"));
    }
}
