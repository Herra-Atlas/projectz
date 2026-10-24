//! Telegram's update shape, narrowed to the two things this app acts on.
//!
//! The Bot API is a general protocol: edited messages, joins, polls, payments. The
//! app asks only for `message` and `callback_query`, and this file is where the rest
//! would be dropped if it arrived anyway. Keeping the parse here means the poller
//! has no branches about JSON and the run path has no idea what Telegram is.

use serde_json::Value;

/// A press on one of the buttons the app drew.
pub struct Action {
    /// The callback id, which has to be acknowledged or the press looks stuck.
    pub id: String,
    /// The opaque payload the button carried.
    pub data: String,
}

/// One thing that happened, as a person on this side of the wire would describe it.
pub struct Update {
    pub id: i64,
    pub user_id: Option<String>,
    pub user_name: String,
    pub chat_id: Option<String>,
    pub text: String,
    pub action: Option<Action>,
    /// Telegram tells us, so a bot-to-bot conversation cannot be started by mistake.
    pub is_bot: bool,
}

/// Reads one update, if it is one of ours.
pub fn parse(value: &Value) -> Option<Update> {
    let id = value.get("update_id")?.as_i64()?;

    if let Some(message) = value.get("message") {
        let from = message.get("from");
        return Some(Update {
            id,
            user_id: from.and_then(|from| from.get("id")).and_then(id_string),
            user_name: from.map(display_name).unwrap_or_default(),
            chat_id: message
                .get("chat")
                .and_then(|chat| chat.get("id"))
                .and_then(id_string),
            // A photo with a caption is still a request; a photo on its own has no
            // text to act on and is left alone rather than answered with an error.
            text: message
                .get("text")
                .or_else(|| message.get("caption"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            action: None,
            is_bot: from
                .and_then(|from| from.get("is_bot"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
        });
    }

    if let Some(query) = value.get("callback_query") {
        let from = query.get("from");
        let data = query
            .get("data")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let callback_id = query
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        return Some(Update {
            id,
            user_id: from.and_then(|from| from.get("id")).and_then(id_string),
            user_name: from.map(display_name).unwrap_or_default(),
            chat_id: query
                .get("message")
                .and_then(|message| message.get("chat"))
                .and_then(|chat| chat.get("id"))
                .and_then(id_string),
            text: String::new(),
            action: (!callback_id.is_empty() && !data.is_empty()).then_some(Action {
                id: callback_id,
                data,
            }),
            is_bot: from
                .and_then(|from| from.get("is_bot"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
        });
    }

    None
}

/// Ids arrive as numbers, but a hand-built update may carry one as a string.
///
/// Both are read rather than one tolerated: the allowlist is a string comparison,
/// and an id that arrived in the other shape must not silently fail to match.
fn id_string(value: &Value) -> Option<String> {
    if let Some(number) = value.as_i64() {
        return Some(number.to_string());
    }
    value.as_str().map(str::to_string)
}

fn display_name(from: &Value) -> String {
    let first = from
        .get("first_name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let last = from
        .get("last_name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let name = format!("{first} {last}").trim().to_string();
    if !name.is_empty() {
        return name;
    }
    from.get("username")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_message_carries_its_sender_and_chat() {
        let update = parse(&json!({
            "update_id": 9,
            "message": {
                "message_id": 3,
                "from": { "id": 12345, "is_bot": false, "first_name": "Ada", "last_name": "L" },
                "chat": { "id": 12345, "type": "private" },
                "text": "hello",
            },
        }))
        .expect("message");
        assert_eq!(update.id, 9);
        assert_eq!(update.user_id.as_deref(), Some("12345"));
        assert_eq!(update.chat_id.as_deref(), Some("12345"));
        assert_eq!(update.user_name, "Ada L");
        assert_eq!(update.text, "hello");
        assert!(update.action.is_none());
        assert!(!update.is_bot);
    }

    /// A button press arrives as a callback, and its data is the whole message.
    #[test]
    fn a_press_carries_its_callback_and_data() {
        let update = parse(&json!({
            "update_id": 10,
            "callback_query": {
                "id": "cb-1",
                "from": { "id": 7, "is_bot": false, "first_name": "Bo" },
                "data": "ap:run-1:y",
                "message": { "message_id": 4, "chat": { "id": 7 } },
            },
        }))
        .expect("callback");
        let action = update.action.expect("action");
        assert_eq!(action.id, "cb-1");
        assert_eq!(action.data, "ap:run-1:y");
        assert_eq!(update.chat_id.as_deref(), Some("7"));
    }

    /// A bot cannot talk to the bot, and an update with neither shape is ignored
    /// rather than guessed at.
    #[test]
    fn bots_and_unknown_updates_are_left_alone() {
        let bot = parse(&json!({
            "update_id": 11,
            "message": { "from": { "id": 1, "is_bot": true }, "chat": { "id": 1 }, "text": "x" },
        }))
        .expect("message");
        assert!(bot.is_bot);
        assert!(parse(&json!({ "update_id": 12, "edited_message": {} })).is_none());
    }

    /// An id in the other JSON shape still matches the allowlist, which is a
    /// string comparison.
    #[test]
    fn an_id_is_read_in_either_shape() {
        assert_eq!(id_string(&json!(42)).as_deref(), Some("42"));
        assert_eq!(id_string(&json!("42")).as_deref(), Some("42"));
    }
}
