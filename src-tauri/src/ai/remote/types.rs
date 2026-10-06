use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Endpoint {
    pub id: String,
    pub name: String,
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub models: Vec<String>,
    /// Models the user has switched off for the pickers.
    ///
    /// Carried alongside `models` rather than merged into it, because the two are
    /// read by different callers: a picker wants the available models, and
    /// Settings wants to show the switched-off ones so they can be switched back
    /// on. A single merged list would force one of them to filter.
    #[serde(default)]
    pub disabled_models: Vec<String>,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct EndpointInfo {
    pub id: String,
    pub name: String,
    pub base_url: String,
    /// Models currently available, i.e. the ones the user has not switched off.
    ///
    /// A disabled model is absent rather than flagged, so a picker cannot show
    /// one by forgetting to check. Settings reads `disabled_models` to be able to
    /// offer it back.
    pub models: Vec<String>,
    /// Models the user has switched off. Kept so the place that manages the
    /// switch is also the place that can undo it -- a disabled model filtered
    /// out of the only list would be unreachable afterwards.
    pub disabled_models: Vec<String>,
    pub enabled: bool,
    pub has_api_key: bool,
}

/// True when the base URL already names its API version somewhere in its path.
///
/// OpenAI-compatible providers are not consistent about where the version sits.
/// Some put it last (`.../openai/v1`, `.../paas/v4`), others put it before a
/// product segment (`.../v1beta/openai`). Testing only the final segment misread
/// the second shape and appended a second version, producing
/// `.../v1beta/openai/v1/models`, which 404s on a provider that would otherwise
/// work. Matching any `v`-then-digit path segment covers both without needing to
/// know a particular provider's layout.
fn names_its_own_version(base: &str) -> bool {
    base.split('/').any(|segment| {
        segment
            .strip_prefix('v')
            .is_some_and(|digits| digits.starts_with(|c: char| c.is_ascii_digit()))
    })
}

/// Builds a request URL from a provider's configured base.
///
/// Only appends `/v1` when the base has not already declared a version, so
/// `https://api.groq.com/openai/v1` and `https://generativelanguage.googleapis.com/v1beta/openai`
/// both resolve to the right chat endpoint.
pub fn endpoint_url(base_url: &str, path: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if names_its_own_version(base) {
        format!("{base}/{path}")
    } else {
        format!("{base}/v1/{path}")
    }
}

/// The provider's model list, for reading capabilities and the picker.
pub fn models_url(base_url: &str) -> String {
    endpoint_url(base_url, "models")
}

/// The provider's OpenAI-compatible chat completions endpoint.
pub fn chat_url(base_url: &str) -> String {
    endpoint_url(base_url, "chat/completions")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every base a real provider uses, and the URL each must resolve to. A
    /// version the app did not know about once produced
    /// `.../v1beta/openai/v1/models`, which 404s on a provider that works.
    #[test]
    fn builds_the_right_url_for_every_base_style() {
        let cases = [
            // Already versioned: nothing is appended.
            (
                "https://api.groq.com/openai/v1",
                "https://api.groq.com/openai/v1/models",
            ),
            (
                "https://api.groq.com/openai/v1/",
                "https://api.groq.com/openai/v1/models",
            ),
            (
                "https://api.z.ai/api/paas/v4",
                "https://api.z.ai/api/paas/v4/models",
            ),
            (
                "https://generativelanguage.googleapis.com/v1beta/openai",
                "https://generativelanguage.googleapis.com/v1beta/openai/models",
            ),
            (
                "https://generativelanguage.googleapis.com/v1beta/openai/",
                "https://generativelanguage.googleapis.com/v1beta/openai/models",
            ),
            // No version at all: v1 is supplied.
            ("https://api.mistral.ai", "https://api.mistral.ai/v1/models"),
            (
                "https://api.cerebras.ai",
                "https://api.cerebras.ai/v1/models",
            ),
            (
                "https://integrate.api.nvidia.com/",
                "https://integrate.api.nvidia.com/v1/models",
            ),
        ];
        for (base, expected) in cases {
            assert_eq!(models_url(base), expected, "wrong URL for {base}");
        }
    }

    /// A bare number or a word that happens to start with `v` is not a version.
    #[test]
    fn does_not_mistake_a_word_for_a_version() {
        assert!(!names_its_own_version("https://api.example.com/api/vision"));
        assert!(!names_its_own_version("https://api.example.com/vllm"));
        // No trailing segment at all, or an empty base, both need a version.
        assert!(!names_its_own_version("https://api.example.com"));
        assert_eq!(
            models_url("https://api.example.com"),
            "https://api.example.com/v1/models"
        );
    }

    #[test]
    fn chat_and_models_share_the_same_base_resolution() {
        let base = "https://generativelanguage.googleapis.com/v1beta/openai";
        assert_eq!(
            chat_url(base),
            "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"
        );
        assert_eq!(
            models_url(base),
            "https://generativelanguage.googleapis.com/v1beta/openai/models"
        );
    }
}
