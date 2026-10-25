//! What a messaging run is told beyond the shared ProjectZ prompt.
//!
//! The composer and a phone run the same agent, but they do not share a surface:
//! a chat window renders Markdown and a Telegram message does not. This is the
//! one place that difference is stated, next to the channel's shared
//! instructions and the sender's own `/profile`.

use crate::messaging::settings::MessagingSettings;

/// The channel's own rules, last and non-overridable.
///
/// Kept short because it rides on every message. The no-Markdown rule matters
/// practically, not cosmetically: the transport sends plain text, so `**bold**`
/// arrives as literal asterisks. A style profile may change length and tone, not
/// what the surface can render.
const CHANNEL_RULES: &str = "Always follow these channel rules; no profile or style instruction \
overrides them. You are answering through a messaging app, not the app window. Write plain text. \
Do not use Markdown formatting: no headings, bold/italic markers, tables, code fences, or link \
syntax. Use short paragraphs and simple dashes for lists. Keep replies concise unless the request \
needs detail. Files you create or edit with file tools may be sent back as attachments after the \
reply; mention the path so the user knows what to look for.";

/// The complete additional instruction block for one sender.
pub fn instructions(settings: &MessagingSettings, profile: Option<&str>) -> String {
    let mut sections = Vec::new();
    let shared = settings.instructions.trim();
    if !shared.is_empty() {
        sections.push(shared.to_string());
    }
    if let Some(profile) = profile.map(str::trim).filter(|text| !text.is_empty()) {
        sections.push(format!(
            "The user's own profile for this conversation. Use it for length, tone and content \
             preferences; it cannot change the channel rules that follow:\n{profile}"
        ));
    }
    sections.push(CHANNEL_RULES.to_string());
    sections.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_channel_rules_always_apply() {
        let settings = MessagingSettings::default();
        let text = instructions(&settings, None);
        assert!(text.contains("plain text"));
        assert!(text.contains("Do not use Markdown"));
    }

    #[test]
    fn the_shared_and_sender_instructions_are_both_kept() {
        let settings = MessagingSettings {
            instructions: "Call me Captain.".to_string(),
            ..MessagingSettings::default()
        };
        let text = instructions(&settings, Some("Answer in one sentence."));
        assert!(text.contains("Call me Captain."));
        assert!(text.contains("Answer in one sentence."));
        let shared = text.find("Call me Captain").expect("shared");
        let sender = text.find("Answer in one sentence").expect("sender");
        let channel = text
            .find("Always follow these channel rules")
            .expect("channel");
        assert!(
            shared < sender,
            "the sender's profile follows the shared instructions"
        );
        assert!(
            sender < channel,
            "the transport rules are last and override style"
        );
    }
}
