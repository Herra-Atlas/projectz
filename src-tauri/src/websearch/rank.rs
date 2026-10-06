//! Re-orders search candidates by evidence quality before any of them reach the
//! model.
//!
//! The engine fanout returns results in *arrival* order, which on a niche query
//! means whichever scraper answered first wins -- not the page that is actually
//! most relevant. Three signals are already present on every `SearchResult` and
//! were being ignored:
//!
//! - `sources` -- every engine that returned this URL. A page found
//!   independently by five engines is a far stronger signal than one engine's
//!   top hit, and this is the only signal that compares engines to each other.
//! - `engine_rank` -- the position the originating engine gave it, so a Bing #1
//!   is distinguished from a Bing #11.
//! - `content` / `snippet` -- fetched page text, used for lexical overlap with
//!   the query once the pages are in.
//!
//! Ranking deliberately happens *after* fetching. Scoring against `content` is
//! much stronger than scoring against snippets, but `content` is `None` until
//! the page has been downloaded, so ranking first would score on nothing.

use kestrelsearch::SearchResult;

/// Weights for the three ranking signals.
///
/// Agreement dominates because it is the only cross-engine signal: a URL that
/// five engines agree on outranks a slightly better-textured page one engine
/// happened to rank first. `engine_rank` is the tiebreaker within that band.
const AGREEMENT_WEIGHT: f64 = 3.0;
const RANK_WEIGHT: f64 = 1.0;
const LEXICAL_WEIGHT: f64 = 2.0;

/// Score every result and return them best-first.
///
/// Returns the input order unchanged for a single result so the common
/// one-page case costs nothing.
pub fn rank(results: Vec<SearchResult>, query: &str) -> Vec<SearchResult> {
    if results.len() < 2 {
        return results;
    }
    let terms = tokenize(query);
    let mut scored: Vec<(SearchResult, f64)> = results
        .into_iter()
        .map(|result| {
            let score = score(&result, &terms);
            (result, score)
        })
        .collect();
    // Stable so that two results with equal evidence keep engine arrival order
    // rather than being shuffled by an arbitrary tiebreak.
    scored.sort_by(|left, right| right.1.total_cmp(&left.1));
    scored.into_iter().map(|(result, _)| result).collect()
}

fn score(result: &SearchResult, terms: &[String]) -> f64 {
    // `sources` is empty for results that came back from a single provider
    // before fanout merged them, so fall back to 1 rather than scoring zero --
    // a real single-engine hit is weak evidence, not zero evidence.
    let agreement = result.sources.len().max(1) as f64;
    let position = 1.0 / (1.0 + best_engine_rank(result) as f64);
    let lexical = lexical_overlap(result, terms);
    AGREEMENT_WEIGHT * agreement + RANK_WEIGHT * position + LEXICAL_WEIGHT * lexical
}

/// The strongest position any engine gave this URL.
///
/// Taking the minimum rather than the first entry matters: the merged entry
/// lists occurrences in engine-completion order, not by rank, so `sources[0]`
/// is just whichever engine finished first.
fn best_engine_rank(result: &SearchResult) -> usize {
    result
        .sources
        .iter()
        .map(|source| source.rank)
        .min()
        .unwrap_or(result.engine_rank.unwrap_or(10))
}

/// Fraction of query terms present in the fetched text, falling back to the
/// snippet when the page could not be retrieved.
///
/// A low-but-nonzero overlap is normal for a good result (the body may discuss
/// the topic without repeating the exact product name), so this is weighted to
/// break ties rather than to dominate the score.
fn lexical_overlap(result: &SearchResult, terms: &[String]) -> f64 {
    if terms.is_empty() {
        return 0.0;
    }
    let haystack = result
        .content
        .as_deref()
        .filter(|content| !content.trim().is_empty())
        .unwrap_or(&result.snippet)
        .to_lowercase();
    let hits = terms
        .iter()
        .filter(|term| haystack.contains(term.as_str()))
        .count();
    hits as f64 / terms.len() as f64
}

/// Lowercased alphanumeric words of at least two characters.
///
/// Single characters are dropped because they match almost everything and so
/// carry no discriminating power.
fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| term.len() >= 2)
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kestrelsearch::Engine;

    fn result(
        url: &str,
        title: &str,
        snippet: &str,
        content: Option<&str>,
        sources: Vec<(Engine, usize)>,
    ) -> SearchResult {
        SearchResult {
            title: title.into(),
            url: url.into(),
            display_url: url.into(),
            snippet: snippet.into(),
            content: content.map(str::to_string),
            bm25_score: None,
            engine: None,
            query: None,
            engine_rank: None,
            sources: sources
                .into_iter()
                .map(|(engine, rank)| kestrelsearch::SourceOccurrence {
                    engine,
                    query: "commandcode".into(),
                    rank,
                })
                .collect(),
        }
    }

    #[test]
    fn cross_engine_agreement_outranks_a_single_strong_hit() {
        // The reported failure: one engine's irrelevant #1 versus a page five
        // engines independently agreed on.
        let lone = result(
            "https://allbiz.com/ck-west",
            "Ck West Investments",
            "Generates USD 140,000",
            None,
            vec![(Engine::Bing, 1)],
        );
        let agreed = result(
            "https://commandcode.ai",
            "CommandCode",
            "CommandCode AI coding workspace",
            Some("CommandCode is an AI coding workspace"),
            vec![
                (Engine::Bing, 9),
                (Engine::Yahoo, 7),
                (Engine::Qwant, 4),
                (Engine::Mojeek, 5),
                (Engine::Ecosia, 6),
            ],
        );

        let ranked = rank(vec![lone, agreed], "commandcode");
        assert_eq!(ranked[0].url, "https://commandcode.ai");
    }

    #[test]
    fn lexical_overlap_breaks_ties_between_equal_agreement() {
        let matching = result(
            "https://a.example",
            "CommandCode",
            "CommandCode AI workspace",
            Some("about CommandCode"),
            vec![(Engine::Bing, 2)],
        );
        let unrelated = result(
            "https://b.example",
            "Home Loans",
            "Mortgage rates",
            Some("mortgage rates today"),
            vec![(Engine::Bing, 2)],
        );

        let ranked = rank(vec![unrelated, matching], "commandcode");
        assert_eq!(ranked[0].url, "https://a.example");
    }

    #[test]
    fn best_engine_rank_is_the_minimum_not_the_first_occurrence() {
        // Sources are merged in engine-completion order, so a fast engine
        // reporting rank 11 can appear before a slower one reporting rank 1.
        let result = result(
            "https://c.example",
            "T",
            "s",
            None,
            vec![(Engine::Bing, 11), (Engine::Qwant, 1)],
        );
        assert_eq!(best_engine_rank(&result), 1);
    }

    #[test]
    fn a_single_result_is_returned_unchanged() {
        let only = result("https://only.example", "T", "s", None, vec![]);
        assert_eq!(rank(vec![only.clone()], "q"), vec![only]);
    }

    #[test]
    fn tokenizer_drops_single_characters() {
        assert_eq!(tokenize("Qwen 3 a model"), vec!["qwen", "model"]);
    }
}
