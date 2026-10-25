//! Files in and out of a messaging channel.
//!
//! # One conversion, the same one the composer uses
//!
//! An inbound document becomes a `ChatAttachment` in the shape `ai::types` already
//! understands: images stay bytes, PDF/Word/Excel keep their extracted text, and
//! text/code is sent as text. The model therefore does not learn a second way of
//! receiving a file just because it came from a phone.
//!
//! # Agent files are written down
//!
//! An attachment's bytes alone cannot be edited. Every Telegram id has a fixed
//! app-data workspace (`<app data>/telegram/<id>/workspace/`); the original is
//! written under its `uploads/<turn>/` folder and the prompt names that relative
//! path in Agent mode. The agent can then read, edit and save the file with its
//! ordinary tools; anything it changes is sent back after the reply. Chat mode has
//! no tools, but the file still lands in the same per-person folder so the data
//! does not move between modes.
//!
//! # Outbound files are the run's, not a guess
//!
//! Only successful file-writing tool calls are candidates. The path comes from the
//! same summary the transcript draws, is resolved again through `resolve_within`,
//! and is sent once per run. A model cannot name an arbitrary file in its prose and
//! have it uploaded.

use std::collections::BTreeSet;
use std::path::Path;

use base64::Engine;
use serde_json::Value;

use crate::ai::tools::resolve_within;
use crate::ai::types::{ChatAttachment, ChatEvent};
use crate::messaging::adapter::Channel;
use crate::messaging::types::{Inbound, InboundFile, OutboundFile};
use crate::messaging::{storage, Messaging};

/// Matches the composer and the request builder. A larger file would be accepted
/// here and then refused at the request, after the upload had been paid for.
const MAX_ATTACHMENT_BYTES: usize = 15 * 1024 * 1024;
/// At most this many changed files are returned from one reply.
const MAX_OUTBOUND_FILES: usize = 8;

/// One inbound turn after its files have been fetched and made usable.
pub struct PreparedMessage {
    /// What the model is sent: the caption, then factual file paths for Agent.
    pub prompt: String,
    /// What the transcript stores: the caption plus readable paths, never base64.
    pub stored: String,
    pub attachments: Vec<ChatAttachment>,
    /// This sender's fixed app-data root, which the run has to adopt.
    pub workspace: std::path::PathBuf,
}

/// Fetches and converts every file on an inbound message.
///
/// A file-only message keeps an empty caption. No instruction is invented here:
/// the model receives the file and decides to ask what the person wants, which is
/// the behaviour the person asked for.
pub async fn prepare_inbound(
    hub: &Messaging,
    channel: &dyn Channel,
    inbound: &Inbound,
    agent: bool,
    user_id: &str,
) -> Result<PreparedMessage, String> {
    let base = inbound.text.trim().to_string();
    // The fixed app-data workspace for this Telegram id. The open ProjectZ folder
    // is deliberately not involved: a remote person gets their own root, and the
    // root does not change from message to message.
    let storage_base = storage::base(hub)?;
    let workspace = storage::workspace(&storage_base, user_id).await?;
    if inbound.files.is_empty() {
        return Ok(PreparedMessage {
            prompt: base.clone(),
            stored: base,
            attachments: Vec::new(),
            workspace,
        });
    }
    if !channel.capabilities().files {
        return Err("This channel cannot receive files.".to_string());
    }

    let turn = uuid::Uuid::new_v4().to_string();
    let stt_allowed = hub.settings().user(user_id).allow_stt_start;

    let mut prompt = base.clone();
    let mut stored = base;
    let mut attachments = Vec::new();
    for file in &inbound.files {
        if file
            .size
            .is_some_and(|size| size > MAX_ATTACHMENT_BYTES as u64)
        {
            return Err(format!("{} is larger than 15 MB.", file.name));
        }
        let downloaded = channel
            .download(file)
            .await
            .map_err(|error| format!("Could not fetch {}: {error}", file.name))?;
        if downloaded.bytes.len() > MAX_ATTACHMENT_BYTES {
            return Err(format!("{} is larger than 15 MB.", file.name));
        }

        let name = safe_name(&downloaded.name);
        let relative = format!("uploads/{turn}/{name}");
        let path = resolve_within(&workspace, &relative)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| format!("Could not save {name}: {error}"))?;
        }
        tokio::fs::write(&path, &downloaded.bytes)
            .await
            .map_err(|error| format!("Could not save {name}: {error}"))?;

        // Voice and audio go through the same selected STT model as the
        // microphone. The transcript becomes the user's words for this turn;
        // the raw audio is not attached to the model.
        if is_audio(file) {
            if !stt_allowed {
                return Err("You don't have access to start the speech-to-text engine.".to_string());
            }
            let transcript = crate::audio::stt::transcribe::voice(
                hub.app(),
                hub.database().as_ref(),
                downloaded.bytes.clone(),
            )
            .await
            .map_err(|error| format!("Could not transcribe {}: {error}", file.name))?;
            if !prompt.is_empty() {
                prompt.push_str("\n\n");
            }
            if !stored.is_empty() {
                stored.push_str("\n\n");
            }
            prompt.push_str(&transcript);
            stored.push_str(&transcript);
            continue;
        }

        let mime = downloaded.mime_type.clone();
        let convert_path = path.clone();
        let convert_name = name.clone();
        let converted = tokio::task::spawn_blocking(move || {
            attachment_from(&convert_name, &mime, &downloaded.bytes, Some(&convert_path))
        })
        .await
        .map_err(|error| format!("Could not read {name}: {error}"))??;
        attachments.push(converted);

        if !prompt.is_empty() {
            prompt.push_str("\n\n");
        }
        if !stored.is_empty() {
            stored.push_str("\n\n");
        }
        if agent {
            prompt.push_str(&format!(
                "Attached file: {name}\nWorkspace path: {relative}"
            ));
            stored.push_str(&format!("Attached file: {relative}"));
        } else {
            prompt.push_str(&format!("Attached file: {name}"));
            stored.push_str(&format!("Attached file: {name}"));
        }
    }

    Ok(PreparedMessage {
        prompt,
        stored,
        attachments,
        workspace,
    })
}

/// Whether Telegram sent audio rather than a document or a still image.
///
/// Voice notes always are; ordinary audio attachments are flagged by their media
/// type. The STT decoder currently accepts Ogg Opus, which is what a Telegram
/// voice note uses.
pub fn is_audio(file: &InboundFile) -> bool {
    file.mime_type.starts_with("audio/")
        || Path::new(&file.name)
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                matches!(
                    extension.to_ascii_lowercase().as_str(),
                    "ogg" | "oga" | "opus" | "mp3" | "wav" | "m4a" | "flac"
                )
            })
}

/// A file name that cannot escape the folder it is written into.
///
/// The path component is taken first, then characters Windows reserves are
/// replaced. The extension survives, because extraction picks the reader by it.
fn safe_name(name: &str) -> String {
    let base = Path::new(name)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    let mut cleaned: String = base
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || matches!(character, '.' | '-' | '_' | ' ') {
                character
            } else {
                '_'
            }
        })
        .collect();
    cleaned = cleaned.trim_matches(['.', ' ']).to_string();
    if cleaned.is_empty() {
        cleaned = "file".to_string();
    }
    cleaned.chars().take(120).collect()
}

/// Converts downloaded bytes into the attachment shape a run already accepts.
fn attachment_from(
    name: &str,
    mime_type: &str,
    bytes: &[u8],
    path: Option<&Path>,
) -> Result<ChatAttachment, String> {
    let extension = Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if matches!(
        mime_type,
        "image/png" | "image/jpeg" | "image/webp" | "image/gif"
    ) || matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif")
    {
        // The transport's media type wins when it named one: a document can be an
        // image with no extension at all, and the bytes must keep their real type.
        let media = match mime_type {
            "image/png" | "image/jpeg" | "image/webp" | "image/gif" => mime_type,
            _ => crate::documents::image_media_type(Path::new(name)).unwrap_or("image/jpeg"),
        };
        return Ok(ChatAttachment {
            name: name.to_string(),
            mime_type: media.to_string(),
            data_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
        });
    }

    if matches!(extension.as_str(), "pdf" | "docx" | "xlsx") {
        let path = path.ok_or_else(|| format!("{name} could not be prepared for reading."))?;
        let text = crate::documents::extract(path)
            .ok_or_else(|| format!("{name} is not a document this app can read."))?
            .map_err(|error| format!("Could not read {name}: {error}"))?;
        if text.trim().is_empty() {
            return Err(format!(
                "{name} has no extractable text. Scanned documents are not supported yet."
            ));
        }
        return Ok(ChatAttachment {
            name: name.to_string(),
            mime_type: "text/plain".to_string(),
            data_base64: base64::engine::general_purpose::STANDARD.encode(text.as_bytes()),
        });
    }

    if is_text_name(&extension) {
        if std::str::from_utf8(bytes).is_err() {
            return Err(format!("{name} is not valid UTF-8 text."));
        }
        return Ok(ChatAttachment {
            name: name.to_string(),
            mime_type: mime_for_text(&extension).to_string(),
            data_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
        });
    }

    Err(format!(
        "{name} is not a supported attachment. Send an image, PDF, Word, Excel, or text/code file."
    ))
}

fn is_text_name(extension: &str) -> bool {
    matches!(
        extension,
        "txt"
            | "log"
            | "md"
            | "markdown"
            | "json"
            | "csv"
            | "html"
            | "htm"
            | "xml"
            | "yaml"
            | "yml"
            | "rs"
            | "py"
            | "js"
            | "jsx"
            | "ts"
            | "tsx"
            | "css"
            | "toml"
            | "sql"
            | "sh"
    )
}

fn mime_for_text(extension: &str) -> &'static str {
    match extension {
        "json" => "application/json",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "xml" => "application/xml",
        "yaml" | "yml" => "text/yaml",
        "rs" => "text/x-rust",
        "py" => "text/x-python",
        "md" | "markdown" => "text/markdown",
        _ => "text/plain",
    }
}

/// The workspace-relative path of a file a successful tool call changed.
///
/// Reads the same summary fields the transcript draws. A failure and a cached call
/// are not changes; a move is identified by its destination.
pub fn changed_file(event: &ChatEvent) -> Option<String> {
    if event.kind != "tool_result" {
        return None;
    }
    let tool = event.text.as_deref()?;
    let metrics = event.metrics.as_ref()?;
    if metrics.get("failed").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let detail = metrics.get("detail").and_then(Value::as_str)?.trim();
    if detail.is_empty() {
        return None;
    }
    match tool {
        "write_file" | "edit_file" | "edit_lines" | "write_document" => Some(detail.to_string()),
        "move_file" => detail
            .rsplit_once(" → ")
            .map(|(_, destination)| destination.trim().to_string()),
        _ => None,
    }
}

/// Sends the files a run changed, once each, inside the workspace.
pub async fn send_changed(channel: &dyn Channel, chat: &str, changed: BTreeSet<String>) {
    if !channel.capabilities().files {
        return;
    }
    let Some(root) = crate::ai::tools::workspace::root() else {
        return;
    };
    let mut sent = 0usize;
    for relative in changed {
        if sent >= MAX_OUTBOUND_FILES {
            let _ = channel
                .send(
                    chat,
                    &format!(
                        "The reply changed more than {MAX_OUTBOUND_FILES} files; the rest are in the workspace."
                    ),
                    &[],
                )
                .await;
            break;
        }
        let Ok(path) = resolve_within(&root, &relative) else {
            continue;
        };
        if !path.is_file() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("file")
            .to_string();
        match channel
            .send_file(
                chat,
                &OutboundFile {
                    path: path.clone(),
                    name: name.clone(),
                },
            )
            .await
        {
            Ok(_) => sent += 1,
            Err(error) => {
                let _ = channel
                    .send(chat, &format!("Could not send {name}: {error}"), &[])
                    .await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_cannot_escape_its_turn_folder() {
        assert_eq!(safe_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_name("na?me.txt"), "na_me.txt");
        assert_eq!(safe_name("   "), "file");
    }

    #[test]
    fn text_attachments_keep_their_media_type() {
        let attachment = attachment_from("notes.md", "text/markdown", b"# hi", None).expect("text");
        assert_eq!(attachment.mime_type, "text/markdown");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(attachment.data_base64)
            .expect("base64");
        assert_eq!(decoded, b"# hi");
    }

    #[test]
    fn an_unsupported_binary_is_refused_by_name() {
        let error =
            attachment_from("song.mp3", "audio/mpeg", b"ID3", None).expect_err("unsupported");
        assert!(error.contains("song.mp3"), "{error}");
    }

    fn changed(kind: &str, tool: &str, detail: &str, failed: bool) -> ChatEvent {
        ChatEvent {
            run_id: "run".into(),
            session_id: None,
            sequence: 1,
            kind: kind.into(),
            text: Some(tool.into()),
            error: None,
            metrics: Some(serde_json::json!({ "detail": detail, "failed": failed })),
        }
    }

    /// Only a successful write is a candidate, and a move is its destination.
    #[test]
    fn only_a_successful_changed_file_is_sent_back() {
        assert_eq!(
            changed_file(&changed("tool_result", "edit_file", "src/a.rs", false)),
            Some("src/a.rs".into())
        );
        assert_eq!(
            changed_file(&changed("tool_result", "write_file", "src/a.rs", true)),
            None
        );
        assert_eq!(
            changed_file(&changed("tool_result", "read_file", "src/a.rs", false)),
            None
        );
        assert_eq!(
            changed_file(&changed("tool_result", "move_file", "a.rs → b.rs", false)),
            Some("b.rs".into())
        );
    }
}
