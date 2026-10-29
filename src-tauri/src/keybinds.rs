//! Shortcuts the operating system owns, and the one settings record behind them.
//!
//! Push-to-talk has to work while ProjectZ is behind another window -- that is
//! what a shortcut is for -- so the registration cannot live in the page. The
//! plugin holds the OS hook and this module holds the choice: one record, one
//! accelerator, and one event the composer listens for.
//!
//! The press is emitted rather than acted on here. Starting a recording is the
//! composer's business (it owns the draft, the send rule and the session), and a
//! second copy of that logic in the backend would be a second answer to "what
//! does pressing the microphone mean".

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::database::Database;

/// The settings key, beside `app.stt` and `app.tts`.
pub const KEY: &str = "app.keybinds";

/// What the composer listens for. One event for one shortcut.
pub const PUSH_TO_TALK_EVENT: &str = "keybind-push-to-talk";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Keybinds {
    /// An accelerator string (`Ctrl+Shift+Space`), or nothing for "no shortcut".
    #[serde(default)]
    pub push_to_talk: Option<String>,
}

fn database(app: &AppHandle) -> std::sync::Arc<Database> {
    app.state::<std::sync::Arc<Database>>().inner().clone()
}

pub fn load(app: &AppHandle) -> Keybinds {
    database(app)
        .setting(KEY)
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

fn save(app: &AppHandle, keybinds: &Keybinds) -> Result<(), String> {
    let value = serde_json::to_value(keybinds)
        .map_err(|error| format!("The shortcut could not be stored: {error}"))?;
    database(app).set_setting(KEY, &value)
}

/// Replaces every registered shortcut with what `keybinds` says.
///
/// Unregister first, then register: a shortcut that changed must not leave the
/// old accelerator live, and one accelerator cannot be registered twice.
pub fn apply(app: &AppHandle, keybinds: &Keybinds) -> Result<(), String> {
    let manager = app.global_shortcut();
    let _ = manager.unregister_all();
    let Some(accelerator) = keybinds
        .push_to_talk
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    manager
        .register(accelerator)
        .map_err(|error| format!("\"{accelerator}\" cannot be registered as a shortcut: {error}"))
}

/// The stored shortcut, re-registered at launch.
///
/// A registration that fails here is logged rather than fatal: a shortcut the OS
/// refuses must not stop the app from opening, and the page reports the same
/// failure when the user tries to set it again.
pub fn restore(app: &AppHandle) {
    if let Err(error) = apply(app, &load(app)) {
        eprintln!("keybinds: {error}");
    }
}

/// The handler the plugin runs for every registered accelerator.
pub fn on_shortcut(app: &AppHandle, state: ShortcutState) {
    // Pressed only: the released half would start and stop a recording in the
    // same breath, which is a microphone that never opens.
    if state == ShortcutState::Pressed {
        let _ = app.emit(PUSH_TO_TALK_EVENT, ());
    }
}

#[tauri::command]
pub fn keybinds_get(app: AppHandle) -> Keybinds {
    load(&app)
}

/// Stores an accelerator, and registers it before saying it was stored.
///
/// Refused at the field rather than at the next press: an accelerator the OS
/// will not take is a typo or another program's, and both are better learned
/// here than when the user tries to talk.
#[tauri::command]
pub fn keybinds_set(app: AppHandle, push_to_talk: Option<String>) -> Result<Keybinds, String> {
    let mut next = load(&app);
    next.push_to_talk = push_to_talk
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    apply(&app, &next)?;
    save(&app, &next)?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::Keybinds;

    /// A record written before a shortcut existed loads with none rather than
    /// failing into the defaults of a different shape.
    #[test]
    fn a_record_without_a_shortcut_loads() {
        let keybinds: Keybinds = serde_json::from_str("{}").unwrap();
        assert_eq!(keybinds.push_to_talk, None);
    }

    #[test]
    fn a_shortcut_round_trips_in_camel_case() {
        let keybinds = Keybinds {
            push_to_talk: Some("Ctrl+Shift+Space".to_string()),
        };
        let json = serde_json::to_string(&keybinds).unwrap();
        assert!(json.contains("\"pushToTalk\""));
        let back: Keybinds = serde_json::from_str(&json).unwrap();
        assert_eq!(back.push_to_talk.as_deref(), Some("Ctrl+Shift+Space"));
    }
}
