use serde_json::Value;

use super::tool_calls::StreamedToolCalls;

#[derive(Debug, Default)]
pub struct StreamedChunk {
    pub completion_id: Option<String>,
    pub content: Option<String>,
    pub reasoning: Option<String>,
    pub finish_reason: Option<String>,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    /// Portion of `prompt_tokens` served from the provider's prompt cache.
    ///
    /// Read but not requested anywhere else in the app: it is the only honest
    /// measure of how effective the cache-stable-prefix work is, and it is what
    /// the statistics page should be able to show.
    pub cached_tokens: Option<u64>,
    pub prompt_eval_tokens_per_second: Option<f64>,
    pub generation_tokens_per_second: Option<f64>,
}

pub fn parse_chunk(value: &Value, tool_calls: &mut StreamedToolCalls) -> Option<StreamedChunk> {
    let choice = value.pointer("/choices/0");
    let delta = choice.and_then(|choice| choice.get("delta"));
    if let Some(calls) = delta
        .and_then(|delta| delta.get("tool_calls"))
        .and_then(Value::as_array)
    {
        tool_calls.append(calls);
    }
    let reasoning = delta.and_then(reasoning_delta);
    Some(StreamedChunk {
        completion_id: value.get("id").and_then(Value::as_str).map(str::to_owned),
        content: delta
            .and_then(|delta| delta.get("content"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        reasoning,
        finish_reason: choice
            .and_then(|choice| choice.get("finish_reason"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        prompt_tokens: value
            .pointer("/usage/prompt_tokens")
            .and_then(Value::as_u64),
        completion_tokens: value
            .pointer("/usage/completion_tokens")
            .and_then(Value::as_u64),
        cached_tokens: cached_tokens(value),
        prompt_eval_tokens_per_second: value
            .pointer("/timings/prompt_per_second")
            .and_then(Value::as_f64),
        generation_tokens_per_second: value
            .pointer("/timings/predicted_per_second")
            .and_then(Value::as_f64),
    })
}

/// Reads cached prompt tokens from whichever shape a provider reports.
///
/// Providers disagree on both the name and the nesting, and all of these appear
/// in the wild among OpenAI-compatible endpoints:
///
/// - `prompt_tokens_details.cached_tokens` (OpenAI)
/// - `usage.cached_tokens` (DeepSeek, several gateways)
/// - `usage.cache_read_input_tokens` (Anthropic)
/// - `prompt_tokens_details.cache_read_tokens` (Anthropic via OpenAI-compat)
///
/// Returns `None` rather than zero when absent, so "the provider reported no
/// cache" is never mistaken for "the provider cached nothing".
fn cached_tokens(value: &Value) -> Option<u64> {
    [
        "/usage/prompt_tokens_details/cached_tokens",
        "/usage/cached_tokens",
        "/usage/cache_read_input_tokens",
        "/usage/prompt_tokens_details/cache_read_tokens",
        "/usage/cache_creation_input_tokens",
    ]
    .iter()
    .find_map(|pointer| value.pointer(pointer).and_then(Value::as_u64))
}

fn reasoning_delta(delta: &Value) -> Option<String> {
    ["reasoning", "reasoning_content", "thinking"]
        .iter()
        .find_map(|key| delta.get(*key).and_then(Value::as_str).map(str::to_owned))
        .or_else(|| {
            delta.get("reasoning")?.as_array().map(|blocks| {
                blocks
                    .iter()
                    .filter_map(|block| block.get("text").and_then(Value::as_str))
                    .collect()
            })
        })
        .filter(|text: &String| !text.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cached_of(usage: Value) -> Option<u64> {
        cached_tokens(&json!({ "usage": usage }))
    }

    #[test]
    fn reads_every_shape_a_provider_reports() {
        // OpenAI
        assert_eq!(
            cached_of(json!({ "prompt_tokens_details": { "cached_tokens": 1200 } })),
            Some(1200)
        );
        // DeepSeek and several gateways
        assert_eq!(cached_of(json!({ "cached_tokens": 900 })), Some(900));
        // Anthropic native
        assert_eq!(
            cached_of(json!({ "cache_read_input_tokens": 700 })),
            Some(700)
        );
        // Anthropic through an OpenAI-compatible surface
        assert_eq!(
            cached_of(json!({ "prompt_tokens_details": { "cache_read_tokens": 400 } })),
            Some(400)
        );
    }

    #[test]
    fn a_provider_that_reports_nothing_reads_as_absent_not_zero() {
        // The distinction matters: zero would average into a hit-rate figure as
        // though the provider had answered and cached nothing.
        assert_eq!(cached_of(json!({ "prompt_tokens": 5000 })), None);
        assert_eq!(cached_of(json!({})), None);
    }

    #[test]
    fn a_zero_cache_read_is_kept_as_a_real_answer() {
        // Zero here is the provider saying "I cached none of this", which is a
        // measurement. It must not be swallowed as if the field were missing.
        assert_eq!(cached_of(json!({ "cached_tokens": 0 })), Some(0));
    }
}
