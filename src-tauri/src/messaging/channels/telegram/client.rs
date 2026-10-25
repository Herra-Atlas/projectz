//! The Bot API, and nothing else.
//!
//! # Long polling, deliberately
//!
//! The app is a desktop program: it has no public hostname, no certificate and no
//! relay. A webhook would need all three, plus a second process running whenever
//! the app does. `getUpdates` holds one HTTPS request open and answers the moment a
//! message arrives, so the idle cost is one connection and the latency is a normal
//! round trip.
//!
//! # No client-level timeout
//!
//! The polling request is *supposed* to take most of a minute; a total timeout on
//! the client would cut every idle poll. Each call carries its own bound instead,
//! and the long poll's is larger than the server-side `timeout` it asks for.

use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::messaging::types::{Button, SentMessage};

const API: &str = "https://api.telegram.org";
/// How long Telegram may hold an idle poll open, in seconds.
pub const LONG_POLL_SECONDS: u64 = 45;
/// The bound on one long poll, above the server's own hold so a reply is not cut.
const LONG_POLL_TIMEOUT: Duration = Duration::from_secs(LONG_POLL_SECONDS + 15);
/// The bound on everything else, because a send that has not answered in twenty
/// seconds has not worked.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// The largest file Telegram will accept from a bot. Anything larger would spend
/// the upload and then be refused, so it is checked before the request.
const MAX_UPLOAD_BYTES: u64 = 50 * 1024 * 1024;
/// The largest file Telegram will serve to a bot.
const MAX_DOWNLOAD_BYTES: u64 = 20 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
pub struct BotUser {
    pub id: i64,
    pub username: Option<String>,
    pub first_name: Option<String>,
}

impl BotUser {
    /// The name a person would recognise, preferring the handle.
    pub fn label(&self) -> String {
        match &self.username {
            Some(username) => format!("@{username}"),
            None => self
                .first_name
                .clone()
                .unwrap_or_else(|| self.id.to_string()),
        }
    }
}

pub struct TelegramClient {
    token: String,
    http: reqwest::Client,
}

impl TelegramClient {
    pub fn new(token: &str) -> Result<Self, String> {
        let token = token.trim();
        if token.is_empty() || token.chars().any(char::is_whitespace) {
            return Err("That does not look like a bot token".to_string());
        }
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .user_agent(concat!("projectz/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| format!("Could not prepare the Telegram connection: {error}"))?;
        Ok(Self {
            token: token.to_string(),
            http,
        })
    }

    /// A transport error without the bot token that sits in every API URL.
    ///
    /// `reqwest` includes the request URL in its display text, so passing that
    /// through would put the credential in a settings field, a log or a chat.
    fn redact(&self, error: impl std::fmt::Display) -> String {
        error.to_string().replace(&self.token, "<bot token>")
    }

    /// One Bot API call.
    ///
    /// Telegram answers `{ ok, result, description, error_code }` with HTTP 200 even
    /// for a refusal, so the body is the only place the truth is; a non-2xx status
    /// still carries one and is read rather than discarded.
    async fn call(&self, method: &str, body: Value, timeout: Duration) -> Result<Value, String> {
        let url = format!("{API}/bot{}/{method}", self.token);
        let response = self
            .http
            .post(url)
            .timeout(timeout)
            .json(&body)
            .send()
            .await
            .map_err(|error| format!("Telegram is unreachable: {}", self.redact(error)))?;
        let payload: Value = response.json().await.unwrap_or_else(|_| Value::Null);
        if payload.get("ok").and_then(Value::as_bool) == Some(true) {
            return Ok(payload.get("result").cloned().unwrap_or(Value::Null));
        }
        let description = payload
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("Telegram refused the request");
        Err(description.to_string())
    }

    pub async fn get_me(&self) -> Result<BotUser, String> {
        let result = self
            .call("getMe", json!({}), Duration::from_secs(15))
            .await?;
        serde_json::from_value(result).map_err(|error| format!("Unexpected getMe answer: {error}"))
    }

    /// Reads pending updates, holding the request open while there are none.
    ///
    /// `offset` is the first update id to accept, which is also how Telegram is told
    /// everything before it was handled.
    pub async fn get_updates(
        &self,
        offset: i64,
        timeout_seconds: u64,
    ) -> Result<Vec<Value>, String> {
        let result = self
            .call(
                "getUpdates",
                json!({
                    "offset": offset,
                    "timeout": timeout_seconds,
                    // Only the two shapes this app acts on: a message, and a press on
                    // one of its own buttons. Everything else is dropped by Telegram
                    // rather than delivered and ignored.
                    "allowed_updates": ["message", "callback_query"],
                }),
                LONG_POLL_TIMEOUT,
            )
            .await?;
        Ok(result.as_array().cloned().unwrap_or_default())
    }

    /// Fetches a file the user sent.
    ///
    /// Telegram names a file with `file_id` and then serves its bytes from a
    /// second URL carrying the bot token. Both halves live here so the core never
    /// learns a token or a Telegram URL.
    pub async fn download_file(&self, file_id: &str) -> Result<Vec<u8>, String> {
        let result = self
            .call(
                "getFile",
                json!({ "file_id": file_id }),
                Duration::from_secs(20),
            )
            .await?;
        let path = result
            .get("file_path")
            .and_then(Value::as_str)
            .ok_or_else(|| "Telegram did not return a file path".to_string())?;
        if result
            .get("file_size")
            .and_then(Value::as_u64)
            .is_some_and(|size| size > MAX_DOWNLOAD_BYTES)
        {
            return Err(
                "The file is larger than Telegram will let a bot download (20 MB).".to_string(),
            );
        }

        let response = self
            .http
            .get(format!("{API}/file/bot{}/{path}", self.token))
            .timeout(Duration::from_secs(60))
            .send()
            .await
            .map_err(|error| format!("Could not download the file: {}", self.redact(error)))?;
        if !response.status().is_success() {
            return Err(format!(
                "Telegram refused the file download ({})",
                response.status()
            ));
        }
        if response
            .content_length()
            .is_some_and(|size| size > MAX_DOWNLOAD_BYTES)
        {
            return Err(
                "The file is larger than Telegram will let a bot download (20 MB).".to_string(),
            );
        }
        let bytes = response.bytes().await.map_err(|error| {
            format!("Could not read the downloaded file: {}", self.redact(error))
        })?;
        if bytes.len() as u64 > MAX_DOWNLOAD_BYTES {
            return Err(
                "The file is larger than Telegram will let a bot download (20 MB).".to_string(),
            );
        }
        Ok(bytes.to_vec())
    }

    /// Sends one file as a document, which preserves its bytes and its name.
    ///
    /// Documents rather than photos even for images: a photo is re-encoded and its
    /// name disappears, which is wrong for a file the agent edited.
    pub async fn send_document(
        &self,
        chat_id: &str,
        path: &std::path::Path,
        name: &str,
    ) -> Result<SentMessage, String> {
        let metadata = tokio::fs::metadata(path)
            .await
            .map_err(|error| format!("Could not read {name}: {error}"))?;
        if metadata.len() > MAX_UPLOAD_BYTES {
            return Err(format!(
                "{name} is larger than Telegram's 50 MB upload limit."
            ));
        }
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|error| format!("Could not read {name}: {error}"))?;
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(name.to_string())
            .mime_str(media_type(name))
            .map_err(|error| format!("Could not prepare {name}: {error}"))?;
        let form = reqwest::multipart::Form::new()
            .text("chat_id", chat_id.to_string())
            .part("document", part);
        let response = self
            .http
            .post(format!("{API}/bot{}/sendDocument", self.token))
            .timeout(Duration::from_secs(120))
            .multipart(form)
            .send()
            .await
            .map_err(|error| format!("Telegram is unreachable: {}", self.redact(error)))?;
        let payload: Value = response.json().await.unwrap_or_else(|_| Value::Null);
        if payload.get("ok").and_then(Value::as_bool) == Some(true) {
            let result = payload.get("result").cloned().unwrap_or(Value::Null);
            let id = result
                .get("message_id")
                .and_then(Value::as_i64)
                .map(|id| id.to_string())
                .unwrap_or_default();
            return Ok(SentMessage { id });
        }
        Err(payload
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("Telegram refused the file")
            .to_string())
    }

    /// Removes a message the app sent.
    pub async fn delete_message(&self, chat_id: &str, message_id: &str) -> Result<(), String> {
        match self
            .call(
                "deleteMessage",
                json!({ "chat_id": chat_id, "message_id": message_id }),
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(error)
                if error.contains("message to delete not found")
                    || error.contains("message is not found") =>
            {
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    pub async fn send_message(
        &self,
        chat_id: &str,
        text: &str,
        buttons: &[Button],
    ) -> Result<SentMessage, String> {
        let mut body = json!({ "chat_id": chat_id, "text": text });
        if let Some(keyboard) = keyboard(buttons) {
            body["reply_markup"] = keyboard;
        }
        let result = self.call("sendMessage", body, REQUEST_TIMEOUT).await?;
        let id = result
            .get("message_id")
            .and_then(Value::as_i64)
            .map(|id| id.to_string())
            .unwrap_or_default();
        Ok(SentMessage { id })
    }

    /// Rewrites a message already sent.
    ///
    /// `message is not modified` is a success: the core edits on a timer and a
    /// large model can finish a step between two of them, so an edit that changes
    /// nothing is the ordinary case rather than a failure.
    pub async fn edit_message(
        &self,
        chat_id: &str,
        message_id: &str,
        text: &str,
        buttons: &[Button],
    ) -> Result<(), String> {
        let mut body = json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "text": text,
        });
        if let Some(keyboard) = keyboard(buttons) {
            body["reply_markup"] = keyboard;
        }
        match self.call("editMessageText", body, REQUEST_TIMEOUT).await {
            Ok(_) => Ok(()),
            Err(error) if error.contains("message is not modified") => Ok(()),
            Err(error) => Err(error),
        }
    }

    pub async fn send_typing(&self, chat_id: &str) -> Result<(), String> {
        self.call(
            "sendChatAction",
            json!({ "chat_id": chat_id, "action": "typing" }),
            Duration::from_secs(10),
        )
        .await?;
        Ok(())
    }

    pub async fn answer_callback(
        &self,
        callback_id: &str,
        text: Option<&str>,
    ) -> Result<(), String> {
        let mut body = json!({ "callback_query_id": callback_id });
        if let Some(text) = text {
            body["text"] = json!(text);
        }
        self.call("answerCallbackQuery", body, Duration::from_secs(10))
            .await?;
        Ok(())
    }
}

/// The media type Telegram should label an uploaded document with.
///
/// Applies only to the metadata; the bytes are sent unchanged. An unknown
/// extension is a generic binary rather than a guess that could make a client
/// try to render something it cannot.
fn media_type(name: &str) -> &'static str {
    match name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("pdf") => "application/pdf",
        Some("txt") | Some("log") => "text/plain",
        Some("md") | Some("markdown") => "text/markdown",
        Some("json") => "application/json",
        Some("csv") => "text/csv",
        Some("html") | Some("htm") => "text/html",
        Some("xml") => "application/xml",
        Some("zip") => "application/zip",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        _ => "application/octet-stream",
    }
}

/// One button per row, so a long label is readable instead of squeezed.
fn keyboard(buttons: &[Button]) -> Option<Value> {
    if buttons.is_empty() {
        return None;
    }
    let rows: Vec<Value> = buttons
        .iter()
        .map(|button| {
            json!([{
                "text": button.label,
                "callback_data": button.data,
            }])
        })
        .collect();
    Some(json!({ "inline_keyboard": rows }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A token is refused before any request, so an obviously wrong paste is
    /// reported at the field rather than as a connection failure.
    #[test]
    fn an_empty_or_spaced_token_is_refused_without_a_request() {
        assert!(TelegramClient::new("").is_err());
        assert!(TelegramClient::new("   ").is_err());
        assert!(TelegramClient::new("123:abc def").is_err());
        assert!(TelegramClient::new("123456:ABC-DEF").is_ok());
    }

    /// No buttons means no `reply_markup` at all: sending an empty keyboard is
    /// accepted but redraws an existing one, which is a change nobody asked for.
    #[test]
    fn an_empty_keyboard_is_absent_rather_than_empty() {
        assert!(keyboard(&[]).is_none());
    }

    /// Each button gets its own row, which keeps a long label readable.
    #[test]
    fn buttons_become_rows() {
        let keyboard = keyboard(&[
            Button::new("Allow", "ap:1:y"),
            Button::new("Deny", "ap:1:n"),
        ])
        .expect("keyboard");
        let rows = keyboard["inline_keyboard"].as_array().expect("rows");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][0]["text"], "Allow");
        assert_eq!(rows[1][0]["callback_data"], "ap:1:n");
    }

    /// A transport error names the URL, and the URL contains the credential.
    #[test]
    fn a_transport_error_loses_the_token() {
        let client = TelegramClient::new("123456:ABC-DEF").expect("client");
        let error = format!(
            "error sending request for url (https://api.telegram.org/bot{}/getMe)",
            "123456:ABC-DEF"
        );
        let redacted = client.redact(error);
        assert!(!redacted.contains("123456:ABC-DEF"));
        assert!(redacted.contains("<bot token>"));
    }

    /// A bot's identity is its handle when it has one, because that is what a
    /// person recognises in the settings page.
    #[test]
    fn a_bot_is_named_by_its_handle() {
        let user = BotUser {
            id: 1,
            username: Some("projectz_bot".into()),
            first_name: Some("ProjectZ".into()),
        };
        assert_eq!(user.label(), "@projectz_bot");
        let unnamed = BotUser {
            id: 7,
            username: None,
            first_name: None,
        };
        assert_eq!(unnamed.label(), "7");
    }
}
