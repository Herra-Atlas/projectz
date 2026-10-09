use serde::{Deserialize, Serialize};

/// A reasoning level, as the provider itself spells it.
///
/// This is a string rather than an enum because providers genuinely disagree:
/// one expects `none`, another expects `off`, another lists `minimal` and
/// `xhigh`. A fixed set cannot satisfy all of them, and sending a level a
/// provider does not accept fails the whole request with HTTP 400. The value the
/// frontend picked comes from the provider's own capability list and is passed
/// through unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReasoningEffort(pub String);

impl Default for ReasoningEffort {
    fn default() -> Self {
        ReasoningEffort("medium".to_string())
    }
}

impl ReasoningEffort {
    /// The value to send, or `None` when no level should be sent at all.
    ///
    /// A model that reports no reasoning support gets no field rather than a
    /// guessed one, because an unknown field is rejected as firmly as a wrong
    /// value.
    pub fn wire_value(&self) -> Option<&str> {
        let value = self.0.trim();
        if value.is_empty() {
            None
        } else {
            Some(value)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    pub run_id: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub session_title: Option<String>,
    pub messages: Vec<ChatMessage>,
    pub endpoint_id: Option<String>,
    #[serde(default)]
    pub local_model: bool,
    pub model: Option<String>,
    #[serde(default)]
    pub attachments: Vec<ChatAttachment>,
    #[serde(default)]
    pub web_search_enabled: bool,
    /// Whether this run may call tools. `Chat` sends no `tools` array at all.
    ///
    /// Defaults to `Chat` rather than `Agent`: a frontend that has not been
    /// rebuilt keeps today's behaviour, and a run that claims Agent mode without
    /// naming a model still gets no tools from the registry.
    #[serde(default)]
    pub mode: crate::ai::tools::ToolMode,
    /// How much the agent may do without asking, for this run.
    ///
    /// Sent on the request rather than read from `app.agent_permission`, because
    /// the level belongs to the conversation. Reading the global setting meant a
    /// session that showed `Ask` could run under whatever level the last-changed
    /// conversation had stored, so reads ran unprompted despite the UI.
    ///
    /// Defaults to `Ask`, the safe direction, for a frontend that has not been
    /// rebuilt.
    #[serde(default)]
    pub permission: crate::ai::tools::PermissionMode,
    /// Absent from older frontends, which sent a boolean instead. `true` matches
    /// the old enabled state and `false` matches the old disabled one, so a
    /// frontend that has not been rebuilt still runs.
    #[serde(default, deserialize_with = "deserialize_reasoning_effort")]
    pub reasoning: ReasoningEffort,
    /// Which skills to apply to this reply, as ids from the `skills` table.
    ///
    /// The instructions themselves travel in the message list, built by the
    /// frontend from the same `skills_enabled` list it offered them from and stored
    /// in the transcript like any other message -- which is what keeps them applying
    /// to every later turn, and keeps them inside the cached prefix rather than
    /// re-sent after it. Rust reads these only to count the use.
    ///
    /// Ids rather than names, so a rename between the picker and the send cannot
    /// silently apply a different skill.
    ///
    /// Defaults to empty so a frontend that has not been rebuilt sends no skills
    /// rather than failing the whole request on an unknown field.
    #[serde(default)]
    pub skill_ids: Vec<String>,
    /// Which capabilities this run may use.
    ///
    /// Absent means everything, so a frontend that never sends it keeps exactly
    /// the behaviour it had. A job sends its own set, and because it rides on the
    /// approval gate the same value is inherited by every sub-agent the run
    /// spawns, which is the requirement: a job's agents may do what the job may.
    #[serde(default)]
    pub access: Option<crate::ai::tools::AccessSet>,
}

/// Accepts a level string, the legacy boolean, or nothing at all, so a frontend
/// that has not been rebuilt still works instead of losing the whole request.
///
/// A boolean has no way to express a level, so `true` becomes the historical
/// default and `false` becomes the historical off value.
fn deserialize_reasoning_effort<'de, D>(deserializer: D) -> Result<ReasoningEffort, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Legacy {
        Effort(String),
        Enabled(bool),
        /// Wrapped so `null` has a variant to land in: an explicit `null` still reaches
        /// the deserializer, because `#[serde(default)]` only covers an absent key.
        Missing(#[allow(unused)] Option<bool>),
    }

    Ok(match Legacy::deserialize(deserializer)? {
        Legacy::Effort(effort) => ReasoningEffort(effort),
        Legacy::Enabled(true) => ReasoningEffort::default(),
        Legacy::Enabled(false) => ReasoningEffort("none".to_string()),
        Legacy::Missing(_) => ReasoningEffort::default(),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatAttachment {
    pub name: String,
    pub mime_type: String,
    pub data_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatResult {
    pub run_id: String,
    pub content: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatEvent {
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub sequence: u64,
    pub kind: String,
    pub text: Option<String>,
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn effort_of(reasoning: serde_json::Value) -> ReasoningEffort {
        serde_json::from_value::<ChatRequest>(serde_json::json!({
            "run_id": "r", "messages": [], "endpoint_id": null, "model": null, "reasoning": reasoning,
        }))
        .unwrap()
        .reasoning
    }

    /// A level arrives exactly as the provider spelled it, whichever dialect that
    /// is, because the app no longer decides the vocabulary for the provider.
    #[test]
    fn a_level_arrives_verbatim_in_any_dialect() {
        for value in [
            "none", "off", "minimal", "low", "medium", "high", "xhigh", "max",
        ] {
            assert_eq!(effort_of(value.into()).0, value);
        }
    }

    /// An older frontend still sends a boolean, and a missing field must not fail the
    /// whole request; both fall back rather than erroring.
    #[test]
    fn accepts_the_legacy_boolean_and_defaults_to_medium() {
        assert_eq!(effort_of(true.into()).0, "medium");
        assert_eq!(effort_of(false.into()).0, "none");
        assert_eq!(effort_of(serde_json::Value::Null).0, "medium");
    }

    /// A missing field must not fail the request either.
    #[test]
    fn a_missing_level_falls_back_to_the_default() {
        let request: ChatRequest = serde_json::from_value(serde_json::json!({
            "run_id": "r", "messages": [], "endpoint_id": null, "model": null,
        }))
        .unwrap();
        assert_eq!(request.reasoning.0, "medium");
    }

    /// Skills arrive as ids the runtime resolves against the table. An older
    /// frontend sends no field at all, which has to mean "no skills" rather than
    /// failing the request on an unknown key -- the same reasoning as the level
    /// above, applied to the one field that arrived after it.
    #[test]
    fn skills_default_to_empty_and_round_trip_ids() {
        let bare: ChatRequest = serde_json::from_value(serde_json::json!({
            "run_id": "r", "messages": [], "endpoint_id": null, "model": null,
        }))
        .unwrap();
        assert!(bare.skill_ids.is_empty());

        let chosen: ChatRequest = serde_json::from_value(serde_json::json!({
            "run_id": "r", "messages": [], "endpoint_id": null, "model": null,
            "skill_ids": ["a", "b"],
        }))
        .unwrap();
        assert_eq!(chosen.skill_ids, vec!["a", "b"]);
    }
}
