//! A throwaway dump of a reply assist could not read.
//!
//! **Temporary. Delete this file, its `pub mod debug;` line and the two calls in `decide.rs`
//! when the failure it was written for is understood.**
//!
//! # Why it exists
//!
//! `the assist reply could not be read` says nothing about what the model actually sent. A
//! provider that wraps its JSON in prose, answers with two objects, puts the answer in a
//! reasoning field, or stops on `max_tokens` halfway through one are four different problems
//! with four different fixes, and the log line is identical for all of them. The raw text is
//! what tells them apart, and it is the one thing the log cannot carry.
//!
//! # What it is careful about
//!
//! Three things, because a diagnostic that damages what it is diagnosing is worse than none:
//!
//! - **Only on a reply that arrived and could not be read.** A transport failure has nothing to
//!   dump that its own error does not say, and a successful request writes nothing at all -- so
//!   an install that works never grows a directory of its own conversations.
//! - **Nothing goes wrong that the request can feel.** Every failure here is dropped on the
//!   floor: the file is for a person reading it later, and the run is the thing that matters.
//! - **No credentials.** The endpoint's key is never written: only its name, its id and where it
//!   is, because those are what identify which provider answered without being a secret.
//!
//! # Where the files go
//!
//! The OS temp directory, under `projectz-assist-debug/` -- not beside the database, because
//! these are for a person debugging this right now rather than state the app keeps, and the
//! system already knows how to clean that up. The path is logged as it is written, so nobody
//! has to guess where to look.

use std::path::PathBuf;

use crate::ai::remote::client::ReportedUsage;
use crate::ai::remote::types::Endpoint;

use super::settings::AssistSettings;

/// The folder under the temp directory.
const DIRECTORY: &str = "projectz-assist-debug";

/// One attempt that came back unreadable, with everything needed to explain it.
pub struct Failure<'a> {
    /// Which try this was: the request is sent twice before its failure is final.
    pub attempt: u32,
    /// What was wrong with the reply, in the parser's own words.
    pub why: &'a str,
    /// How the thinking switch was set for this attempt, in words: `allowed`, `suppressed - thinking
    /// is off in Settings`, or `suppressed - the salvage attempt after a truncated reply`.
    ///
    /// A note rather than a flag because suppression has two reasons now, and a file that calls both
    /// of them "the salvage attempt" misleads the person reading it: with thinking off, the *first*
    /// attempt is suppressed too, and nothing was salvaged.
    pub thinking: &'a str,
    /// Why the model stopped, in the provider's own word. `length` is the one that explains a reply
    /// that is thinking rather than an answer.
    pub finish_reason: Option<&'a str>,
    pub endpoint: &'a Endpoint,
    pub model: &'a str,
    pub settings: &'a AssistSettings,
    /// The two messages exactly as they were sent, byte for byte.
    pub system_prompt: &'a str,
    pub user_prompt: &'a str,
    /// The text the reply was read from, exactly as it arrived.
    pub reply: &'a str,
    /// The provider's whole response, which is where the thinking and the stop reason are.
    pub payload: &'a serde_json::Value,
    pub usage: &'a ReportedUsage,
}

/// Writes one dump, and answers with where it went.
///
/// `None` on any failure at all -- no directory, no disk, no permission -- because this exists to
/// explain a request that already failed, and it must not be able to fail one that did not.
pub fn write(failure: &Failure<'_>) -> Option<PathBuf> {
    let directory = std::env::temp_dir().join(DIRECTORY);
    std::fs::create_dir_all(&directory).ok()?;
    // The clock is in the name so two failures do not overwrite each other, and the colons are
    // taken out of it because Windows reads them as a drive separator.
    let stamp = crate::database::utc_now().replace(':', "-");
    let file = directory.join(format!(
        "assist-fail-{stamp}-attempt{}.log",
        failure.attempt
    ));
    std::fs::write(&file, body(failure)).ok()?;
    Some(file)
}

/// The dump itself, as plain text with its own headings.
///
/// Verbatim where it matters and never truncated: the interesting part of a bad reply is usually
/// at the end of it, which is exactly what a cap would cut off.
fn body(failure: &Failure<'_>) -> String {
    let settings = failure.settings;
    let usage = failure.usage;

    let mut text = String::new();
    text.push_str("ProjectZ assist - a reply that arrived and could not be read\n");
    text.push_str("=========================================================\n\n");
    text.push_str(&format!("when      {}\n", crate::database::utc_now()));
    text.push_str(&format!(
        "attempt   {} (the request is sent twice before its failure is final)\n",
        failure.attempt
    ));
    text.push_str(&format!("why       {}\n", failure.why));
    // Two lines that say how the reply came to be what it is, and neither is in the text: a reply
    // cut off at the ceiling and one that stopped on its own read identically in a file, and the
    // switch that was thrown on this attempt is the difference between a repeat and a salvage.
    text.push_str(&format!(
        "finish    {}\n",
        match failure.finish_reason {
            Some("length") => "length - cut off at the token ceiling, not finished".to_string(),
            Some(other) => other.to_string(),
            None => "(the provider reported none)".to_string(),
        }
    ));
    text.push_str(&format!("thinking  {}\n", failure.thinking));
    text.push_str(&format!(
        "provider  {} ({}) at {}\n",
        failure.endpoint.name, failure.endpoint.id, failure.endpoint.base_url
    ));
    text.push_str(&format!("model     {}\n", failure.model));
    text.push_str(&format!(
        "level     {:?} - {} candidates, {} answers, {} reply tokens, {} characters a block\n",
        settings.level,
        settings.max_candidates(),
        settings.max_answers(),
        settings.max_tokens(),
        settings.max_ask_chars
    ));
    text.push_str(&format!(
        "usage     prompt {:?}, completion {:?}, cached {:?}\n",
        usage.prompt_tokens, usage.completion_tokens, usage.cached_tokens
    ));
    text.push_str(&format!("source    {}\n", settings.hint));
    text.push_str(&format!("subjects  {}\n", settings.subjects.join(", ")));

    text.push_str("\n--- system prompt, as sent ---\n");
    text.push_str(failure.system_prompt);
    text.push_str("\n\n--- user prompt, as sent ---\n");
    text.push_str(failure.user_prompt);

    // The text the parser was handed, with its length: a reply that stopped early and one that
    // was never generated look the same in a file, and the figure is what tells them apart.
    text.push_str(&format!(
        "\n\n--- the reply, read as text ({} characters) ---\n",
        failure.reply.chars().count()
    ));
    text.push_str(failure.reply);

    // And the response it came out of, because `content` is only one field of it: the parser's
    // input is the fallback the client chose, while the thinking and the stop reason sit beside
    // it, untouched by anything this app did.
    text.push_str("\n\n--- the provider's whole response ---\n");
    match serde_json::to_string_pretty(failure.payload) {
        Ok(pretty) => text.push_str(&pretty),
        // A payload that cannot be re-serialized is still worth seeing as it is.
        Err(_) => text.push_str(&failure.payload.to_string()),
    }
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint() -> Endpoint {
        Endpoint {
            id: "test".to_string(),
            name: "Test provider".to_string(),
            base_url: "https://example.invalid/v1".to_string(),
            api_key: "sk-secret-not-to-be-written".to_string(),
            models: Vec::new(),
            disabled_models: Vec::new(),
            enabled: true,
        }
    }

    fn payload() -> serde_json::Value {
        serde_json::json!({
            "choices": [{
                "finish_reason": "length",
                "message": {
                    "content": "",
                    "reasoning_content": "the thinking the app never reads when content exists",
                }
            }]
        })
    }

    /// **Everything that explains the failure is in the file, verbatim.** The two prompts as they
    /// were sent, the reply as it arrived, the provider's own response, the model, the level, why
    /// the model stopped and whether this attempt was the salvage one -- because the whole reason
    /// this file exists is that the log line names none of them.
    #[test]
    fn a_dump_carries_the_request_the_reply_and_the_switch() {
        let endpoint = endpoint();
        let settings = AssistSettings::default();
        let payload = payload();
        let path = write(&Failure {
            attempt: 2,
            why: "The model's decision could not be read: trailing characters at line 1 column 34",
            thinking: "suppressed - the salvage attempt after a truncated reply",
            finish_reason: crate::ai::remote::client::finish_reason(&payload),
            endpoint: &endpoint,
            model: "a-model",
            settings: &settings,
            system_prompt: "SYSTEM PROMPT AS SENT",
            user_prompt: "[2] The value of PI?",
            reply: "{\"answers\": []}  and some trailing words",
            payload: &payload,
            usage: &ReportedUsage {
                prompt_tokens: Some(120),
                completion_tokens: Some(80),
                cached_tokens: None,
            },
        })
        .expect("a dump must be written");

        let body = std::fs::read_to_string(&path).expect("the dump must be readable");
        assert!(body.contains("attempt   2"));
        assert!(body.contains("SYSTEM PROMPT AS SENT"));
        assert!(body.contains("[2] The value of PI?"));
        assert!(body.contains("{\"answers\": []}  and some trailing words"));
        assert!(body.contains("trailing characters at line 1 column 34"));
        assert!(body.contains("the thinking the app never reads when content exists"));
        assert!(body.contains("\"finish_reason\": \"length\""));
        // The two facts that are not in the reply: it was cut off, and this attempt had thinking
        // switched off because of it.
        assert!(
            body.contains("finish    length - cut off at the token ceiling"),
            "{body}"
        );
        assert!(body.contains("thinking  suppressed"), "{body}");
        assert!(body.contains("a-model"));
        assert!(body.contains("prompt Some(120)"));

        // The key is never written: a diagnostic that leaks a credential is one nobody can send.
        assert!(
            !body.contains("sk-secret-not-to-be-written"),
            "the endpoint's key must never reach the file"
        );

        let _ = std::fs::remove_file(&path);
    }
}
