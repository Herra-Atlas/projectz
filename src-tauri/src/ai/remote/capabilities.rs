//! What a model can do, learned from its provider's own `/models` response.
//!
//! Providers disagree on both the shape of that response and the vocabulary of
//! reasoning levels, so nothing here is guessed. A field the provider did not
//! send stays `None`, and `None` means *unknown* — never "this model cannot".
//! The frontend treats unknown as "behave exactly as before", which is what
//! keeps a provider we have never seen working.

use serde::Serialize;
use serde_json::Value;

/// Capabilities of one model, as learned from a provider response.
#[derive(Serialize, Clone, Debug, Default)]
pub struct ModelCapabilities {
    pub context_length: Option<i64>,
    pub input_modalities: Option<Vec<String>>,
    pub output_modalities: Option<Vec<String>>,
    /// `None` when the provider said nothing about reasoning.
    pub supports_reasoning: Option<bool>,
    /// The exact strings the provider accepts, when it listed them.
    pub reasoning_values: Option<Vec<String>>,
    /// `None` when the provider said nothing about tool calling.
    ///
    /// Separate from reasoning because they are independent: a model can think
    /// without calling tools, and several local builds take neither.
    pub supports_tools: Option<bool>,
}

/// Reads one entry of a provider's model list.
///
/// Tolerant by design. Providers disagree about naming — an OpenAI-shaped entry
/// carries `id`, other vendors use `name` — so every lookup here is a list of
/// candidates rather than a single fixed path. A provider that reports only a
/// name yields no capabilities, which is not an error: it simply means we have
/// nothing to adapt to.
pub fn parse_capabilities(entry: &Value) -> ModelCapabilities {
    ModelCapabilities {
        context_length: parse_context_length(entry),
        input_modalities: parse_modalities(entry, "input_modalities"),
        output_modalities: parse_modalities(entry, "output_modalities"),
        supports_reasoning: parse_supports_reasoning(entry),
        reasoning_values: parse_reasoning_values(entry),
        supports_tools: parse_supports_tools(entry),
    }
}

/// Whether the provider accepts a `tools` array.
///
/// An explicit flag wins, then the presence of `tools` in `supported_parameters`.
/// Both are treated the way reasoning is: the parameter list is authoritative
/// when it exists, so a model that lists parameters *without* `tools` is
/// reported as not taking tools. That is the case worth catching, because a
/// model sent a `tools` array it ignores will simply chat instead.
fn parse_supports_tools(entry: &Value) -> Option<bool> {
    if let Some(flag) = entry
        .get("supports_tools")
        .or_else(|| entry.get("supportsTools"))
        .or_else(|| entry.get("supports_function_calling"))
        .and_then(Value::as_bool)
    {
        return Some(flag);
    }
    let supported = entry.get("supported_parameters")?.as_array()?;
    let names: Vec<&str> = supported.iter().filter_map(Value::as_str).collect();
    Some(
        names
            .iter()
            .any(|name| name.contains("tool") || name.contains("function")),
    )
}

/// The identifier a provider uses for one of its models.
///
/// `id` is the OpenAI name. Other vendors use `name`, sometimes namespaced
/// (`models/gemini-3.8-flash`), so that prefix is stripped: the same model must
/// resolve to the same stored row whichever shape reported it, and the caller
/// sends the name the user types.
fn model_identifier(entry: &Value) -> Option<String> {
    let raw = ["id", "name", "model", "model_id"]
        .iter()
        .find_map(|key| entry.get(*key).and_then(Value::as_str))?;
    let trimmed = raw.trim();
    let name = trimmed
        .strip_prefix("models/")
        .unwrap_or(trimmed)
        .trim_start_matches('/');
    (!name.is_empty()).then(|| name.to_string())
}

/// Pulls every model out of a `/models` payload.
///
/// Both shapes seen in the wild are accepted: `{ "data": [...] }`, which is the
/// OpenAI convention, and `{ "models": [...] }`, used by vendors that expose a
/// native listing alongside an OpenAI-compatible chat endpoint. Anything else
/// yields nothing rather than an error.
pub fn parse_models(body: &Value) -> Vec<(String, ModelCapabilities)> {
    let entries = ["data", "models"]
        .iter()
        .find_map(|key| body.get(*key).and_then(Value::as_array))
        .or_else(|| body.as_array());
    let Some(entries) = entries else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| {
            let id = model_identifier(entry)?;
            Some((id, parse_capabilities(entry)))
        })
        .collect()
}

/// Context length, wherever this provider chose to put it.
///
/// The names differ by vendor: gateways report `context_length` or
/// `context_window`, and vendors with a native listing report an input token
/// limit instead. All of them mean the same thing here — how much the model can
/// hold — so they are read as one figure.
fn parse_context_length(entry: &Value) -> Option<i64> {
    const PATHS: [&str; 6] = [
        "top_provider.context_length",
        "context_length",
        "context_window",
        "max_context_length",
        "inputTokenLimit",
        "input_token_limit",
    ];
    for path in PATHS {
        let mut cursor = entry;
        let mut found = true;
        for key in path.split('.') {
            match cursor.get(key) {
                Some(next) => cursor = next,
                None => {
                    found = false;
                    break;
                }
            }
        }
        if !found {
            continue;
        }
        if let Some(length) = cursor.as_i64().filter(|value| *value > 0) {
            return Some(length);
        }
        // llama.cpp and some gateways report the context as a number of tokens
        // under `max_tokens` for a single generation, which is not the window.
        if let Some(length) = cursor.as_str().and_then(|value| value.parse().ok()) {
            if length > 0 {
                return Some(length);
            }
        }
    }
    None
}

fn parse_modalities(entry: &Value, key: &str) -> Option<Vec<String>> {
    let list = entry
        .get("architecture")
        .and_then(|architecture| architecture.get(key))
        .or_else(|| entry.get(key))?
        .as_array()?;
    let values: Vec<String> = list
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_lowercase)
        .filter(|value| !value.is_empty())
        .collect();
    // An empty array is the provider saying nothing useful, not saying "none".
    if values.is_empty() {
        None
    } else {
        Some(values)
    }
}

/// Whether the provider accepts a reasoning control at all.
///
/// Three shapes are recognised: a boolean flag, the presence of the parameter in
/// `supported_parameters`, or a non-empty list of effort values.
fn parse_supports_reasoning(entry: &Value) -> Option<bool> {
    if let Some(flag) = entry
        .get("supports_reasoning")
        .or_else(|| entry.get("supportsReasoning"))
        .and_then(Value::as_bool)
    {
        return Some(flag);
    }
    if let Some(supported) = entry.get("supported_parameters").and_then(Value::as_array) {
        let names: Vec<&str> = supported.iter().filter_map(Value::as_str).collect();
        if names.iter().any(|name| name.contains("reasoning")) {
            return Some(true);
        }
        // The parameter list is authoritative: reasoning is absent from it, so
        // this model does not take the field.
        if !names.is_empty() {
            return Some(false);
        }
    }
    parse_reasoning_values(entry).map(|values| !values.is_empty())
}

/// The exact effort strings this model accepts.
///
/// Providers spell the levels differently (`none` on one, `off` on another) and
/// the difference is why a hardcoded level fails with HTTP 400. Reading them
/// from the response is what lets the app send a value the model will accept.
fn parse_reasoning_values(entry: &Value) -> Option<Vec<String>> {
    const PATHS: [&str; 3] = [
        "supported_reasoning_efforts",
        "supported_reasoning",
        "reasoning.efforts",
    ];
    for path in PATHS {
        let mut cursor = entry;
        let mut found = true;
        for key in path.split('.') {
            match cursor.get(key) {
                Some(next) => cursor = next,
                None => {
                    found = false;
                    break;
                }
            }
        }
        if !found {
            continue;
        }
        let values: Vec<String> = cursor
            .as_array()
            .map(|list| {
                list.iter()
                    .filter_map(Value::as_str)
                    .map(|value| value.trim().to_lowercase())
                    .filter(|value| !value.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        if !values.is_empty() {
            return Some(values);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_an_openrouter_shaped_entry() {
        let entry = json!({
            "id": "stepfun/step-3.7-flash:free",
            "architecture": {
                "input_modalities": ["Text", "image"],
                "output_modalities": ["Text"]
            },
            "top_provider": {"context_length": 131072},
            "supported_parameters": ["reasoning_effort", "temperature", "tools"],
            "supported_reasoning_efforts": ["none", "minimal", "low", "medium", "high"]
        });

        let capabilities = parse_capabilities(&entry);

        assert_eq!(capabilities.context_length, Some(131_072));
        assert_eq!(
            capabilities.input_modalities,
            Some(vec!["text".to_string(), "image".to_string()])
        );
        assert_eq!(capabilities.supports_reasoning, Some(true));
        assert_eq!(
            capabilities.reasoning_values.as_deref(),
            Some(
                ["none", "minimal", "low", "medium", "high"]
                    .map(String::from)
                    .as_slice()
            )
        );
    }

    /// The bare OpenAI shape carries no capabilities. Every field must read as
    /// unknown rather than as an absence, so the app keeps its current
    /// behaviour instead of disabling things it simply has no data for.
    #[test]
    fn a_bare_openai_entry_yields_only_unknowns() {
        let entry = json!({"id": "gpt-4o", "object": "model", "owned_by": "openai"});

        let capabilities = parse_capabilities(&entry);

        assert_eq!(capabilities.context_length, None);
        assert_eq!(capabilities.input_modalities, None);
        assert_eq!(capabilities.supports_reasoning, None);
        assert_eq!(capabilities.reasoning_values, None);
        // Nothing here invents a window. The fallback used when a model has
        // never been inspected is a rendering concern and lives in the frontend
        // (`modelCapabilities.ts`), so the composer is the one thing deciding
        // what to assume.
    }

    #[test]
    fn a_model_that_rejects_reasoning_is_reported_as_such() {
        let entry = json!({
            "id": "poolside/laguna-s-2.1-free",
            "supported_parameters": ["temperature", "max_tokens"],
            "top_provider": {"context_length": 256000}
        });

        let capabilities = parse_capabilities(&entry);

        assert_eq!(capabilities.supports_reasoning, Some(false));
        assert_eq!(capabilities.context_length, Some(256_000));
    }

    /// A model that lists parameters without `tools` is reported as not taking
    /// tools. This is the case worth catching: such a model sent a `tools` array
    /// ignores it and answers in prose, so Agent mode would appear to do nothing.
    #[test]
    fn a_model_whose_parameters_exclude_tools_is_reported_as_such() {
        let entry = json!({
            "id": "some/chat-model",
            "supported_parameters": ["temperature", "max_tokens", "reasoning_effort"]
        });

        assert_eq!(parse_capabilities(&entry).supports_tools, Some(false));
    }

    #[test]
    fn tools_in_the_parameter_list_are_read_as_support() {
        for name in ["tools", "tool_choice", "function_calling"] {
            let entry = json!({ "id": "m", "supported_parameters": [name] });
            assert_eq!(
                parse_capabilities(&entry).supports_tools,
                Some(true),
                "{name} was not read as tool support"
            );
        }
    }

    /// An explicit flag wins over the parameter list, for providers that report
    /// both and disagree.
    #[test]
    fn an_explicit_tools_flag_overrides_the_parameter_list() {
        let entry = json!({
            "id": "m",
            "supported_parameters": ["temperature"],
            "supports_tools": true
        });

        assert_eq!(parse_capabilities(&entry).supports_tools, Some(true));
    }

    /// No list and no flag means unknown, not absent. Agent mode stays offered so
    /// an untested provider behaves exactly as it did before this existed.
    #[test]
    fn a_model_that_reports_nothing_about_tools_reads_as_unknown() {
        let entry = json!({ "id": "m", "owned_by": "someone" });
        assert_eq!(parse_capabilities(&entry).supports_tools, None);
    }

    /// Reasoning and tools are independent: a model can think without calling
    /// tools, which is why these are two columns rather than one.
    #[test]
    fn reasoning_support_does_not_imply_tool_support() {
        let entry = json!({
            "id": "m",
            "supported_parameters": ["reasoning_effort"]
        });

        let capabilities = parse_capabilities(&entry);
        assert_eq!(capabilities.supports_reasoning, Some(true));
        assert_eq!(capabilities.supports_tools, Some(false));
    }

    /// A reported window is stored exactly as the provider gave it, small ones
    /// included. `outputTokenLimit` is never substituted for it: that is a
    /// generation cap, and treating it as a context window would understate
    /// how much the model can hold.
    #[test]
    fn a_reported_context_length_is_kept_verbatim() {
        let small = parse_capabilities(&json!({
            "id": "tiny",
            "top_provider": {"context_length": 4096}
        }));
        assert_eq!(small.context_length, Some(4096));

        // A model with an output cap but no reported window stays unknown
        // rather than borrowing the cap as its context.
        let output_capped = parse_capabilities(&json!({
            "id": "capped",
            "outputTokenLimit": 8192
        }));
        assert_eq!(output_capped.context_length, None);

        let absent = parse_capabilities(&json!({"id": "unknown"}));
        assert_eq!(absent.context_length, None);
    }

    #[test]
    fn reads_models_from_a_wrapped_or_bare_array() {
        let wrapped = parse_models(&json!({"data": [
            {"id": "a", "top_provider": {"context_length": 1000}},
            {"id": "b"}
        ]}));
        assert_eq!(wrapped.len(), 2);
        assert_eq!(wrapped[0].1.context_length, Some(1000));
        assert_eq!(wrapped[1].1.context_length, None);

        let bare = parse_models(&json!([{"id": "c"}]));
        assert_eq!(bare.len(), 1);
        assert_eq!(bare[0].0, "c");

        // An unexpected shape yields nothing rather than failing the refresh.
        assert!(parse_models(&json!({"error": "nope"})).is_empty());
    }

    /// Some vendors publish a native listing alongside an OpenAI-compatible
    /// chat endpoint: a `models` array instead of `data`, a `name` instead of an
    /// `id`, and an input token limit instead of a context length. The shape is
    /// different but the meaning is not, so it is read the same way rather than
    /// special-cased for one vendor.
    #[test]
    fn reads_a_native_listing_shaped_like_a_vendors_own_api() {
        let models = parse_models(&json!({
            "models": [
                {
                    "name": "models/gemini-3.8-flash",
                    "displayName": "Gemini 3.8 Flash",
                    "inputTokenLimit": 1048576,
                    "outputTokenLimit": 65536,
                    "supportedGenerationMethods": ["generateContent", "countTokens"]
                },
                {"name": "models/gemini-flash-lite"}
            ]
        }));

        assert_eq!(models.len(), 2);
        // The `models/` namespace is stripped so the name matches what the user
        // types and what a chat request carries.
        assert_eq!(models[0].0, "gemini-3.8-flash");
        assert_eq!(models[0].1.context_length, Some(1_048_576));
        // A model reporting no limit is unknown, not zero.
        assert_eq!(models[1].1.context_length, None);
    }

    /// `outputTokenLimit` is a generation cap, not a context window, so it must
    /// not be mistaken for one.
    #[test]
    fn an_output_limit_is_not_a_context_window() {
        let models = parse_models(&json!({
            "data": [{"id": "x", "outputTokenLimit": 65536}]
        }));
        assert_eq!(models[0].1.context_length, None);
    }

    /// An entry that names a model under an unexpected key still appears in the
    /// list; only its capabilities go unread.
    #[test]
    fn an_entry_without_any_known_name_is_skipped() {
        let models = parse_models(&json!({
            "data": [{"object": "model"}, {"id": "kept"}]
        }));
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].0, "kept");
    }

    #[test]
    fn effort_values_are_lower_cased_and_trimmed() {
        let entry = json!({
            "id": "x",
            "supported_reasoning_efforts": [" Low ", "HIGH", ""]
        });
        assert_eq!(
            parse_reasoning_values(&entry),
            Some(vec!["low".to_string(), "high".to_string()])
        );
    }

    /// An empty list is the provider telling us nothing, so it must not be read
    /// as "this model supports nothing".
    #[test]
    fn an_empty_modality_list_reads_as_unknown() {
        let entry = json!({"id": "x", "architecture": {"input_modalities": []}});
        assert_eq!(parse_modalities(&entry, "input_modalities"), None);
    }
}
