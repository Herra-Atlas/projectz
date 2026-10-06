//! The web search tool, wrapping [`crate::websearch::WebSearchClient`].
//!
//! This was previously a branch inside the streaming loop. Moving it here means
//! the loop has no knowledge of any specific tool, and adding a second one costs
//! a line in `AGENT_TOOLS` rather than a rewrite.
//!
//! It stays out of [`super::AGENT_TOOLS`] for now: web search is opt-in per
//! conversation through the composer's toggle, and mixing it in with the
//! filesystem tools would advertise it in every Agent turn whether or not the
//! user asked for it. The registry gains it only when `web_search_enabled`.

use std::sync::Arc;

use serde_json::Value;

use crate::websearch::WebSearchClient;

use super::{ToolContext, ToolSpec};

/// Runs one search and returns the text the model will read.
///
/// Reporting rather than returning the UI data to the caller is what keeps this
/// an ordinary tool. The alternative -- special-casing search inside the
/// streaming loop -- is the thing this refactor removed.
pub async fn search(query: &str, context: ToolContext) -> Result<String, String> {
    // Cloned rather than moved: `cancelled` is taken by the client call, and the
    // context is still needed afterwards to report the result.
    let output = WebSearchClient::search_context(query, Arc::clone(&context.cancelled)).await?;
    // Queued rather than emitted from here: the loop owns the run's callback and
    // drains the collector, so exactly one place reports events.
    context.report_search(output.clone());
    Ok(output.context)
}

/// Schema and executor for `search_web`.
///
/// Built by a function rather than a `const` because the executor needs to name
/// this module's `search`. The schema strings are still `&'static str`, so the
/// cache guarantee on the `tools` array is unaffected.
pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "search_web",
        // The frequency guidance belongs here rather than in the system prompt:
        // it only applies when this tool is present, and a prompt instruction
        // would still be sent on turns where the tool is absent.
        description: "Search the web when current, time-sensitive, or source-backed information would improve the answer. Do not call this for every message. Use a concise search query based on the user's request.",
        parameters: r#"{
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Concise search query."
                }
            },
            "required": ["query"],
            "additionalProperties": false
        }"#,
        effect: super::Effect::Read,
        // A search query is not a command line. The tool only fetches URLs the
        // engine returned, so there is nothing for the policy to classify.
        command_argument: None,
        execute: |arguments, context| {
            Box::pin(async move {
                let query = query_of(&arguments)?;
                Ok(search(&query, context).await?)
            })
        },
    }
}

/// Page extraction options, shared with the fetch tool.
///
/// One definition because a page that reads one way through `search_web` and
/// another through `web_fetch` would make a model's output depend on which tool
/// it happened to use.
pub(crate) fn fetch_options() -> kestrelsearch::FetchOptions {
    // Fully qualified because this module is also named `websearch`: a relative
    // `super::super` would resolve to this module rather than the crate's.
    crate::websearch::context::fetch_options()
}

/// The query the model asked for.
pub fn query_of(arguments: &Value) -> Result<String, String> {
    arguments
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "search_web requires a non-empty string `query`".to_string())
}
