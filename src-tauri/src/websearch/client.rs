use std::sync::Arc;

use kestrelsearch::{KestrelClient, SearchOptions};
use tokio::sync::OnceCell;

use super::context::{fetch_options, format_search_context};
use super::rank;

const MAX_QUERY_CHARS: usize = 240;
/// Pages whose extracted text reaches the model.
const MAX_FETCHED_PAGES: usize = 5;
/// Pages downloaded and scored before the best `MAX_FETCHED_PAGES` are chosen.
///
/// Wider than the output on purpose: ranking reads fetched text, so candidates
/// must exist before the cut. Costs extra downloads, not extra model tokens.
const MAX_FETCH_CANDIDATES: usize = 10;

/// Engine fanout tuning. The defaults silently produce bad results here.
///
/// `min_results` is an **early stop**, not a target: the fanout cancels the
/// remaining engines as soon as this many unique candidates exist. At 5 with
/// 8 engines racing, whichever two or three respond fastest fill the quota and
/// the rest never vote. Bing and Yahoo are both the fastest scrapers here and
/// both have weak indexes for a niche product name, so they won the race and
/// won the quota. 15 lets every engine contribute before the fanout stops.
///
/// `search_budget` must be >= 15s. Below that threshold Kestrel enables up to
/// two empty-query deadline retries, each adding up to 5s, so a 6s budget was
/// not a fast clean search -- it was a partial one cut off mid-flight. That is
/// why a query returned only three results: the budget expired and whatever
/// survived was taken.
const SEARCH_BUDGET_SECS: u64 = 15;
const SEARCH_CONCURRENCY: usize = 8;
const SEARCH_MIN_RESULTS: usize = 15;

static CLIENT: OnceCell<Result<Arc<KestrelClient>, String>> = OnceCell::const_new();

pub struct WebSearchClient;

impl WebSearchClient {
    pub async fn search_context(
        query: &str,
        cancelled: Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<super::types::WebSearchOutput, String> {
        tokio::select! {
            result = Self::search_context_inner(query) => result,
            _ = wait_for_cancellation(cancelled) => Err("Web search cancelled".into()),
        }
    }

    async fn search_context_inner(query: &str) -> Result<super::types::WebSearchOutput, String> {
        let query = query.trim();
        if query.is_empty() {
            return Err("Web search needs a non-empty query".into());
        }
        let query = query.chars().take(MAX_QUERY_CHARS).collect::<String>();
        let search_query = query.clone();
        let client = CLIENT
            .get_or_init(|| async {
                KestrelClient::with_parser_capacity(2)
                    .map(Arc::new)
                    .map_err(|error| error.to_string())
            })
            .await
            .as_ref()
            .map_err(Clone::clone)?;

        let options = SearchOptions {
            search_budget: Some(std::time::Duration::from_secs(SEARCH_BUDGET_SECS)),
            max_concurrency: SEARCH_CONCURRENCY,
            min_results: Some(SEARCH_MIN_RESULTS),
            ..SearchOptions::default()
        };
        let results = client
            .search_many(&[query], &options)
            .await
            .map_err(|error| error.to_string())?;
        if results.is_empty() {
            return Ok(super::types::WebSearchOutput {
                query: search_query.clone(),
                context: format!("A web search was already performed for: {search_query}. No results were found. Answer the user's request using your existing knowledge and state that search returned no sources. Do not ask the user to provide a query or emit simulated tool calls."),
                results: Vec::new(),
            });
        }
        // A wider candidate set than we ultimately show. Ranking reads the
        // fetched `content`, which is why the trim to MAX_FETCHED_PAGES happens
        // *after* fetching: selecting first would mean ranking on snippets only,
        // and would permanently lock in whichever pages the engines happened to
        // return first.
        let candidates = dedupe_by_domain(results)
            .into_iter()
            .take(MAX_FETCH_CANDIDATES)
            .collect::<Vec<_>>();
        let urls = candidates
            .iter()
            .map(|result| result.url.clone())
            .collect::<Vec<_>>();
        let pages = if urls.is_empty() {
            Vec::new()
        } else {
            client
                .fetch_all_with_budget(
                    &urls,
                    &fetch_options(),
                    std::time::Duration::from_secs(10),
                )
                .await
                .unwrap_or_else(|error| {
                    tracing::warn!(error = %error, "web page extraction failed; using search snippets");
                    vec![None; candidates.len()]
                })
        };

        // Attach the fetched text so the ranker can score on real page content
        // rather than on the search snippet.
        let fetched = candidates
            .into_iter()
            .zip(pages)
            .map(|(mut result, page)| {
                result.content = page.filter(|text| !text.trim().is_empty());
                result
            })
            .collect::<Vec<_>>();

        let selected = rank::rank(fetched, &search_query)
            .into_iter()
            .take(MAX_FETCHED_PAGES)
            .collect::<Vec<_>>();
        // Unwrap the bodies again: `format_search_context` reads them per index
        // and falls back to the snippet when a page is absent.
        let pages = selected
            .iter()
            .map(|result| result.content.clone())
            .collect::<Vec<_>>();

        Ok(format_search_context(search_query, &selected, &pages))
    }
}

async fn wait_for_cancellation(cancelled: Arc<std::sync::atomic::AtomicBool>) {
    while !cancelled.load(std::sync::atomic::Ordering::Relaxed) {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// Keeps at most one result per host, preserving the best-supported one.
///
/// The reported failure was three results from three different business
/// directories that were all the same irrelevant site; one result per host stops
/// a single noisy host from filling the whole context.
///
/// Compared on host rather than registrable domain, which would need a public
/// suffix list to be correct: `github.com/userA` and `github.com/userB` are the
/// same host and must not collapse to one, which simple suffix-stripping gets
/// wrong. The trade-off is that separate subdomains of one publisher each survive.
fn dedupe_by_domain(results: Vec<kestrelsearch::SearchResult>) -> Vec<kestrelsearch::SearchResult> {
    let mut seen = std::collections::HashSet::new();
    results
        .into_iter()
        .filter(|result| {
            let host = host_of(&result.url);
            !host.is_empty() && seen.insert(host)
        })
        .collect()
}

/// Host of a URL, lowercased, or an empty string when it will not parse.
///
/// Falls back to the raw string rather than dropping the result outright, so an
/// unusual URL still competes on its ranking signals.
fn host_of(url: &str) -> String {
    match reqwest::Url::parse(url) {
        Ok(parsed) => parsed.host_str().unwrap_or_default().to_lowercase(),
        Err(_) => url.to_lowercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_one_result_per_host() {
        let results = vec![
            stub("https://commandcode.ai/docs"),
            stub("https://commandcode.ai/blog"),
            stub("https://github.com/commandcodeai"),
        ];
        let deduped = dedupe_by_domain(results);
        let urls = deduped
            .iter()
            .map(|result| result.url.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            urls,
            vec![
                "https://commandcode.ai/docs",
                "https://github.com/commandcodeai"
            ]
        );
    }

    #[test]
    fn same_host_collapses_but_distinct_paths_keep_the_first() {
        let results = vec![
            stub("https://github.com/commandcodeai"),
            stub("https://github.com/other"),
            stub("https://commandcode.ai"),
        ];
        let deduped = dedupe_by_domain(results);
        let urls = deduped
            .iter()
            .map(|result| result.url.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            urls,
            vec!["https://github.com/commandcodeai", "https://commandcode.ai"]
        );
    }

    #[test]
    fn host_parsing_is_case_insensitive() {
        assert_eq!(host_of("https://CommandCode.AI/Docs"), "commandcode.ai");
    }

    fn stub(url: &str) -> kestrelsearch::SearchResult {
        kestrelsearch::SearchResult {
            title: "t".into(),
            url: url.into(),
            display_url: url.into(),
            snippet: String::new(),
            content: None,
            bm25_score: None,
            engine: None,
            query: None,
            engine_rank: None,
            sources: Vec::new(),
        }
    }
}
