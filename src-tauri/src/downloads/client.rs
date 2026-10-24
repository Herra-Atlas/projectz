//! HuggingFace, as this app needs it.
//!
//! Three requests and nothing else: a search, a repository's file tree, and the
//! Piper index. The client is built once per command rather than kept alive --
//! a settings page that is open for a minute does not need a pool held for the
//! rest of the process' life.

use std::time::Duration;

use super::catalog::TreeEntry;

const API: &str = "https://huggingface.co";

/// A search or a tree is a small JSON document; one that has not answered in this
/// long is not going to. The file download has no such limit -- a 4 GB model is
/// allowed to take as long as it takes, and it is streamed rather than buffered.
const TIMEOUT: Duration = Duration::from_secs(30);

pub struct Client {
    http: reqwest::Client,
    /// A second client with no total timeout, for the files themselves.
    ///
    /// The one above must fail fast -- a search that stalls is a page with a
    /// spinner -- but a 4 GB model is allowed to take ten minutes, and sharing
    /// the client would have cut it off at thirty seconds.
    files: reqwest::Client,
}

/// The one field a search result is read for.
#[derive(serde::Deserialize)]
struct RepoHit {
    id: String,
}

impl Client {
    pub fn new() -> Result<Self, String> {
        let agent = concat!("projectz/", env!("CARGO_PKG_VERSION"));
        let http = reqwest::Client::builder()
            // Named rather than left as the default: HuggingFace throttles
            // unidentified clients harder, and this app is one.
            .user_agent(agent)
            .timeout(TIMEOUT)
            .build()
            .map_err(|error| format!("Could not prepare the download client: {error}"))?;
        let files = reqwest::Client::builder()
            .user_agent(agent)
            .build()
            .map_err(|error| format!("Could not prepare the download client: {error}"))?;
        Ok(Self { http, files })
    }

    /// The client for file bodies: same identity, no total timeout.
    pub fn files(&self) -> &reqwest::Client {
        &self.files
    }

    /// The JSON client, for a caller that wants to run several requests at once.
    ///
    /// `reqwest::Client` is a handle around a shared pool, so cloning it is how a
    /// task gets its own copy: a future that borrows the `Client` cannot be
    /// spawned, and owns nothing to keep itself alive.
    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    /// Repositories matching `query` that hold GGUF files, most downloaded first.
    ///
    /// Sorted by downloads because a search for "llama" returns thousands and the
    /// ones worth offering are the ones other people used.
    pub async fn search_gguf(&self, query: &str, limit: usize) -> Result<Vec<String>, String> {
        let mut url = reqwest::Url::parse(&format!("{API}/api/models"))
            .map_err(|error| format!("Could not build the search: {error}"))?;
        url.query_pairs_mut()
            .append_pair("search", query)
            .append_pair("filter", "gguf")
            .append_pair("limit", &limit.to_string())
            .append_pair("sort", "downloads")
            .append_pair("direction", "-1");

        let hits: Vec<RepoHit> = self.get_json(url.as_str()).await?;
        Ok(hits.into_iter().map(|hit| hit.id).collect())
    }

    /// Piper's own index: every voice, its files and their sizes.
    pub async fn piper_index(&self) -> Result<serde_json::Value, String> {
        self.get_json(&format!(
            "{API}/{}/resolve/main/voices.json",
            super::catalog::PIPER_REPO
        ))
        .await
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T, String> {
        get_json(&self.http, url).await
    }
}

/// A repository's file tree, as a free function so a caller can run several at
/// once with an owned `reqwest::Client` in each future.
pub async fn tree(http: &reqwest::Client, repo: &str) -> Result<Vec<TreeEntry>, String> {
    get_json(
        http,
        &format!("{API}/api/models/{repo}/tree/main?recursive=true"),
    )
    .await
}

async fn get_json<T: serde::de::DeserializeOwned>(
    http: &reqwest::Client,
    url: &str,
) -> Result<T, String> {
    let response = http
        .get(url)
        .send()
        .await
        .map_err(|error| format!("HuggingFace did not answer: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "HuggingFace answered {} for {url}",
            response.status().as_u16()
        ));
    }
    response
        .json::<T>()
        .await
        .map_err(|error| format!("HuggingFace sent something unreadable: {error}"))
}
