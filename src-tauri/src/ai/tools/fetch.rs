//! Fetching the contents of a specific URL.
//!
//! `search_web` already retrieves pages, but it retrieves the ones *it* chose.
//! This is for the case where a model has a URL and needs the page behind it:
//! the documentation page it was told about, a source it wants to quote, a file
//! from a repository.
//!
//! Reuses the same extraction the search path uses, so a page reads the same way
//! either way. Exposing it separately is what lets a model go deeper than a
//! search result summary without a second search.

use kestrelsearch::KestrelClient;

use serde_json::Value;

use super::{ToolContext, ToolSpec};

/// Fetch a web page and return its text.
pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "web_fetch",
        description:
            "Fetch a web page and return its text content. Use this when you already have the \
                      URL, for example documentation you were given or a source you want to quote. \
                      For finding pages in the first place, use search_web.",
        parameters: r#"{
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The full http or https URL to fetch."
                }
            },
            "required": ["url"],
            "additionalProperties": false
        }"#,
        effect: super::Effect::Read,
        // Fetching a URL is a read of the world, not of the workspace, so it is
        // not subject to the path containment check and has no command to
        // classify. It is a Read so it may be cached.
        command_argument: None,
        execute: |arguments, context| Box::pin(fetch(arguments, context)),
    }
}

/// Same client the search path uses.
///
/// Built per call rather than shared, because the search client is held in a
/// `OnceCell` for the whole process and this one is used rarely enough that
/// holding a connection pool open for it is not worth the coupling.
async fn fetch(arguments: Value, context: ToolContext) -> Result<String, String> {
    let url = arguments
        .get("url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .ok_or("web_fetch requires a non-empty string `url`")?
        .to_string();

    // Scheme checked before the request rather than after a failure, so a
    // `file://` URL is refused by name instead of surfacing as a confusing
    // transport error. A fetch tool that would read the local disk is not this
    // tool.
    let scheme = url
        .split_once("://")
        .map(|(scheme, _)| scheme.to_lowercase());
    match scheme.as_deref() {
        Some("http" | "https") => {}
        Some(other) => {
            return Err(format!(
                "Only http and https URLs can be fetched, not {other}:"
            ))
        }
        None => {
            return Err(format!(
                "{url} is not a URL; it needs an http:// or https:// prefix"
            ))
        }
    }

    let client = KestrelClient::new().map_err(|error| error.to_string())?;
    let pages = client
        .fetch_all(&[url.clone()], &super::websearch::fetch_options())
        .await
        .map_err(|error| format!("Could not fetch {url}: {error}"))?;

    let page = pages
        .into_iter()
        .flatten()
        .next()
        // A `None` here means the fetcher refused the body: a PDF, an unsupported
        // content type, or a host that blocked the request. Saying which is
        // more useful than returning an empty page that reads as "blank site".
        .ok_or_else(|| {
            format!(
                "{url} returned no readable text. It may not be an HTML page, or the site may have \
                 blocked the request."
            )
        })?;

    if page.trim().is_empty() {
        return Err(format!("{url} loaded but contained no text."));
    }
    // Cancellation is only checked after the fetch: the fetch itself takes up to
    // the timeout in `fetch_options`, and a tool that abandons that would
    // cancel the request for no benefit to the user.
    if context.cancelled.load(std::sync::atomic::Ordering::Relaxed) {
        return Err("Cancelled".into());
    }
    Ok(page)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cancelled() -> ToolContext {
        ToolContext {
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            sink: super::super::Sink::default(),
            database: None,
        }
    }

    /// The scheme check runs before any request, so a `file://` URL is refused
    /// by name. A fetch tool that would read the local disk is not this tool.
    #[test]
    fn a_non_http_scheme_is_refused() {
        for url in [
            "file:///etc/passwd",
            "C:\\Windows\\System32",
            "data:text/html,hello",
        ] {
            let error = block_on(fetch(json!({ "url": url }), cancelled())).expect_err("refused");
            assert!(
                error.contains("http") || error.contains("not a URL"),
                "{url} was not refused clearly: {error}"
            );
        }
    }

    #[test]
    fn a_missing_or_empty_url_is_reported() {
        assert!(block_on(fetch(json!({}), cancelled()))
            .expect_err("missing")
            .contains("url"));
        assert!(block_on(fetch(json!({ "url": "  " }), cancelled()))
            .expect_err("empty")
            .contains("url"));
    }

    #[test]
    fn a_cancelled_run_does_not_return_a_page() {
        let context = ToolContext {
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            sink: super::super::Sink::default(),
            database: None,
        };
        // Never reached over the network: the scheme check fails first, so this
        // only proves the cancelled context is threaded through without panicking.
        assert!(block_on(fetch(json!({ "url": "not-a-url" }), context)).is_err());
    }

    #[test]
    fn the_tool_is_a_read_and_is_not_registered_by_default() {
        // A fetch is a read of the world, and it is opt-in like search rather
        // than part of Agent mode, so it must not appear unasked.
        let spec = super::spec();
        assert_eq!(spec.effect, super::super::Effect::Read);
        assert_eq!(spec.command_argument, None);
        assert!(
            !super::super::registry_for(super::super::ToolMode::Agent, false)
                .names()
                .contains(&"web_fetch")
        );
    }

    fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(future)
    }
}
