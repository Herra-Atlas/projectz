use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct WebSearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct WebSearchOutput {
    pub query: String,
    pub context: String,
    pub results: Vec<WebSearchResult>,
}
