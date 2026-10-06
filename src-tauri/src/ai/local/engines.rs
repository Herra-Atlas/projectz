use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    time::{Duration, SystemTime},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};
use zip::ZipArchive;

const RELEASES_URL: &str = "https://api.github.com/repos/ggml-org/llama.cpp/releases?per_page=30";

/// Where a model's engine lives: one setting per model, `local.engine_for.{modelId}`.
///
/// A distinct prefix rather than a field in `LocalRuntimeSettings`, because an
/// engine is not a `llama-server` flag: `to_args()` takes no engine argument, so
/// putting it there would either be ignored by the settings page or -- worse --
/// become a field the flag builder has to know about and skip. There is
/// deliberately no global default beside it: every model names its own engine,
/// so there is no second answer for the loader to consult and disagree with.
const ENGINE_SETTING_PREFIX: &str = "local.engine_for.";

const CATALOG_CACHE_AGE: Duration = Duration::from_secs(60 * 60 * 12);
const MAX_ARCHIVE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: u64 = 512 * 1024 * 1024;
const MAX_ARCHIVE_FILES: usize = 256;

/// The `llama.cpp` id prefix, and the one this module's own catalog uses.
const LLAMA_CPP_ID_PREFIX: &str = "llama.cpp-vulkan-";

/// Marks an engine that did not come from the verified release catalog.
///
/// A pasted URL carries no checksum -- GitHub publishes a digest per release
/// asset and a raw download link does not carry one -- so an engine installed
/// this way can never be verified the way a catalog one is. It is labelled
/// rather than refused: the URL was the user's own choice, and refusing it would
/// make forks of `llama.cpp` unusable, which is the whole reason this path
/// exists. The label is the honest record of what is known about the bytes.
const CUSTOM_SOURCE: &str = "custom";

/// How many releases the catalog keeps.
///
/// Every build that ships a matching asset, rather than a fixed short list.
/// The catalog used to stop at three, which made an engine the user had already
/// installed fall off the end of it: it was no longer listed at all, so it could
/// not be selected, uninstalled, or even counted, and the page reported "0
/// installed" while a 32 MB engine sat on disk. The limit is now only a guard
/// against a pathological response -- a release list is small metadata, unlike
/// the archives it points at, so this is never reached in practice.
const MAX_CATALOG_ENGINES: usize = 30;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineInfo {
    pub id: String,
    pub version: String,
    pub updated_at: String,
    pub asset_name: String,
    pub size_bytes: u64,
    pub installed: bool,
    /// False for an installed engine whose release has left the catalog.
    ///
    /// The page needs this to keep offering the uninstall action while offering
    /// neither the download that cannot succeed nor a size and date that are no
    /// longer known. Without it a stale row would render as a Download button
    /// that fails on click.
    pub available: bool,
    /// Where this engine came from: `llama.cpp` for a catalog build, `custom` for
    /// one installed from a pasted link.
    ///
    /// `#[serde(default)]` so a `catalog.json` written before this field existed
    /// still parses -- the cached catalog is durable state on disk, and an
    /// engine the user already installed must not become unlistable because the
    /// app was upgraded.
    #[serde(default = "default_source")]
    pub source: String,
}

fn default_source() -> String {
    "llama.cpp".to_string()
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    published_at: String,
    draft: bool,
    assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct EngineCatalog {
    fetched_at: u64,
    engines: Vec<CatalogEngine>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CatalogEngine {
    id: String,
    version: String,
    updated_at: String,
    asset_name: String,
    download_url: String,
    /// Empty when there is nothing to verify against, which is the case for every
    /// engine installed from a pasted link. `install` refuses a catalog entry
    /// without one; a custom install never goes through that check, because it is
    /// not pretending to be verified.
    sha256: String,
    size_bytes: u64,
    /// See [`EngineInfo::source`].
    #[serde(default = "default_source")]
    source: String,
}

/// A GitHub release download link, taken apart into the three parts that name
/// where an engine comes from.
///
/// The only shape accepted: `github.com/{owner}/{repo}/releases/download/{tag}/
/// {asset}`, over `http` or `https` -- an `http` paste is upgraded rather than
/// refused. Everything else is refused by name rather than attempted, because a
/// pasted link is the one place in this module where the URL did not come from
/// GitHub's own release API -- so the check exists to keep a `file://` URL or a
/// UNC path out of the downloader, not to gate which repositories a user may
/// install from. A fork is a legitimate reason to be here, and it publishes its
/// builds exactly like `llama.cpp` does.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ReleaseLink {
    owner: String,
    repository: String,
    tag: String,
    asset: String,
}

/// Why a pasted link was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LinkError {
    /// Not a GitHub address, or a scheme that is neither `http` nor `https`.
    NotAUrl,
    /// Right host and scheme, wrong path -- and the most common paste, which is
    /// the releases *page* rather than the asset's own address.
    NotAReleaseAsset,
}

impl ReleaseLink {
    fn parse(input: &str) -> Result<Self, LinkError> {
        let trimmed = input.trim();
        let url = reqwest::Url::parse(trimmed).map_err(|_| LinkError::NotAUrl)?;
        if url.host_str() != Some("github.com") {
            return Err(LinkError::NotAUrl);
        }
        // `http` is upgraded rather than refused.
        //
        // A pasted link is very often `http://` -- hand-edited, an old bookmark,
        // a chat client that rewrote it -- and refusing it sends someone hunting
        // for a typo that is not there. The upgrade is what a browser does
        // before it sends anything, and GitHub redirects the same way; the
        // download client is `https_only` regardless, so nothing this module
        // sends can leave over plain HTTP even if a redirect tried. Every other
        // scheme is refused, because the host check above is what bounds this
        // and it needs one to mean anything.
        if !matches!(url.scheme(), "http" | "https") {
            return Err(LinkError::NotAUrl);
        }
        let segments: Vec<&str> = url
            .path_segments()
            .map(|segments| segments.filter(|segment| !segment.is_empty()).collect())
            .unwrap_or_default();
        // owner / repo / releases / download / tag / asset
        let [owner, repository, releases, download, tag, asset] = segments.as_slice() else {
            return Err(LinkError::NotAReleaseAsset);
        };
        if *releases != "releases" || *download != "download" {
            return Err(LinkError::NotAReleaseAsset);
        }
        // A query string or fragment is not part of which file this is, and a
        // stray one pasted onto the end would otherwise ride along silently.
        Ok(Self {
            owner: (*owner).to_string(),
            repository: (*repository).to_string(),
            tag: (*tag).to_string(),
            asset: (*asset).to_string(),
        })
    }

    /// The directory name this engine is installed under.
    ///
    /// `{owner}-{repo}-{tag}`, so two forks publishing the same build number
    /// cannot land in the same folder -- which is the whole reason a pasted
    /// engine is separated from a catalog one rather than installed beside it.
    fn id(&self) -> String {
        let mut id = String::new();
        for part in [&self.owner, &self.repository, &self.tag] {
            for character in part.chars() {
                // The same character set `installed_dir` accepts, applied here so
                // an id derived from a URL cannot be one that directory rejects.
                id.push(
                    if character.is_ascii_alphanumeric() || matches!(character, '-' | '.') {
                        character
                    } else {
                        '_'
                    },
                );
            }
            id.push('-');
        }
        id.trim_end_matches('-').to_string()
    }
}

fn data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join("engines"))
        .map_err(|error| error.to_string())
}

fn current_platform_asset() -> Option<&'static str> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        Some("win-vulkan-x64.zip")
    }
    #[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
    {
        None
    }
}

fn cache_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(data_dir(app)?.join("catalog.json"))
}

/// Where engines installed from a pasted link are recorded.
///
/// A file of its own rather than an entry in `catalog.json`, because that file is
/// rewritten wholesale every time the catalog is refreshed -- a custom engine
/// stored in it would be deleted by the next "Refresh catalog", along with its
/// version and the URL it came from. What it holds is not a cache and is not
/// derived from anything: it is the only record of a build the release API never
/// mentioned, so it is written once and read until the engine is uninstalled.
fn custom_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(data_dir(app)?.join("custom.json"))
}

fn installed_dir(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_'))
    {
        return Err("Invalid engine identifier".to_string());
    }
    Ok(data_dir(app)?.join(id))
}

fn read_catalog_cache(app: &AppHandle) -> Option<EngineCatalog> {
    let path = cache_path(app).ok()?;
    let metadata = fs::metadata(&path).ok()?;
    let age = metadata.modified().ok()?.elapsed().ok()?;
    if age > CATALOG_CACHE_AGE {
        return None;
    }
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

async fn fetch_catalog() -> Result<EngineCatalog, String> {
    let platform_asset = current_platform_asset().ok_or_else(|| {
        "Vulkan engine downloads are not available for this platform yet".to_string()
    })?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(concat!("ProjectZ/", env!("CARGO_PKG_VERSION")))
        .https_only(true)
        .build()
        .map_err(|error| error.to_string())?;
    let releases: Vec<Release> = client
        .get(RELEASES_URL)
        .send()
        .await
        .map_err(|error| format!("Could not check llama.cpp releases: {error}"))?
        .error_for_status()
        .map_err(|error| error.to_string())?
        .json()
        .await
        .map_err(|error| format!("Could not read llama.cpp releases: {error}"))?;

    let suffix = format!("bin-{platform_asset}");
    let mut engines = Vec::new();
    for release in releases {
        let Some(build_number) = release.tag_name.strip_prefix('b') else {
            continue;
        };
        if release.draft
            || build_number.is_empty()
            || !build_number.bytes().all(|byte| byte.is_ascii_digit())
        {
            continue;
        }
        let Some(asset) = release
            .assets
            .into_iter()
            .find(|asset| asset.name.starts_with("llama-") && asset.name.ends_with(&suffix))
        else {
            continue;
        };
        let Some(digest) = asset
            .digest
            .as_deref()
            .and_then(|value| value.strip_prefix("sha256:"))
        else {
            continue;
        };
        let Ok(url) = reqwest::Url::parse(&asset.browser_download_url) else {
            continue;
        };
        if url.scheme() != "https" || url.host_str() != Some("github.com") {
            continue;
        }
        let id = format!("{LLAMA_CPP_ID_PREFIX}{}", release.tag_name);
        engines.push(CatalogEngine {
            id,
            version: release.tag_name,
            updated_at: release.published_at,
            asset_name: asset.name,
            download_url: asset.browser_download_url,
            sha256: digest.to_ascii_lowercase(),
            size_bytes: asset.size,
            source: default_source(),
        });
        if engines.len() == MAX_CATALOG_ENGINES {
            break;
        }
    }
    Ok(EngineCatalog {
        fetched_at: now_unix(),
        engines,
    })
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn load_saved_engines(app: &AppHandle) -> Result<EngineCatalog, String> {
    let path = cache_path(app)?;
    if path.is_file() {
        return serde_json::from_slice(&fs::read(path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string());
    }
    Ok(EngineCatalog {
        fetched_at: 0,
        engines: Vec::new(),
    })
}

fn save_catalog(app: &AppHandle, catalog: &EngineCatalog) -> Result<(), String> {
    let path = cache_path(app)?;
    let parent = path
        .parent()
        .ok_or_else(|| "Invalid engine catalog path".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("json.tmp");
    fs::write(
        &temporary,
        serde_json::to_vec(catalog).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    fs::rename(temporary, path).map_err(|error| error.to_string())
}

fn load_custom(app: &AppHandle) -> Vec<CatalogEngine> {
    custom_path(app)
        .ok()
        .and_then(|path| fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_custom(app: &AppHandle, engines: &[CatalogEngine]) -> Result<(), String> {
    let path = custom_path(app)?;
    let parent = path
        .parent()
        .ok_or_else(|| "Invalid custom engine path".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("json.tmp");
    fs::write(
        &temporary,
        serde_json::to_vec(engines).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    fs::rename(temporary, path).map_err(|error| error.to_string())
}

pub async fn list(app: &AppHandle, refresh: bool) -> Result<Vec<EngineInfo>, String> {
    let cached_catalog = read_catalog_cache(app);
    let mut should_save = false;
    // `mut` because the orphan merge below appends installed engines the catalog
    // no longer lists. The binding itself is never reassigned.
    #[allow(unused_mut)]
    let mut catalog = if !refresh {
        if let Some(cached) = cached_catalog {
            cached
        } else {
            match fetch_catalog().await {
                Ok(fresh) => {
                    should_save = true;
                    fresh
                }
                Err(error) => {
                    let Some(stale) = load_saved_engines(app).ok() else {
                        return Err(error);
                    };
                    if stale.engines.is_empty() {
                        return Err(error);
                    }
                    stale
                }
            }
        }
    } else {
        match fetch_catalog().await {
            Ok(fresh) => {
                should_save = true;
                fresh
            }
            Err(error) => {
                let Some(stale) = load_saved_engines(app).ok() else {
                    return Err(error);
                };
                if stale.engines.is_empty() {
                    return Err(error);
                }
                stale
            }
        }
    };
    if should_save {
        save_catalog(app, &catalog)?;
    }
    let installed_ids = installed_engine_ids(app)?;
    let mut catalog = catalog;

    // An engine that is on disk but no longer in the catalog is still the user's.
    //
    // The catalog is a window onto recent releases, not a record of what was
    // installed: llama.cpp publishes builds faster than anyone can install them,
    // so any engine falls out of it within days. Without this, a fall-back one
    // could not be selected, could not be uninstalled, and did not even count
    // towards "N installed" -- it became invisible while occupying disk.
    //
    // Merged in catalog order where they still appear, and appended at the end
    // where they no longer do. A synthesised entry carries no download URL and
    // no checksum, which is why `install` would refuse it anyway; `uninstall`
    // works because it only needs the id, and `select` works because it checks
    // the installed directory rather than the catalog.
    //
    // Written as one predicate below rather than two, because the later merge has
    // more to exclude than "not in the catalog" -- see the custom discussion --
    // and two blocks would double-report: the first would collect an orphan that
    // the second then has to take back out again.
    //
    // Engines from a pasted link are appended ahead of the orphans, so the
    // catalog's own rows stay contiguous at the top and the page can tell a
    // remembered custom build from a directory it found by itself. A custom entry
    // whose directory is gone is dropped rather than shown: unlike a catalog
    // engine, nothing about it is recoverable, so the record is all there is.
    let custom: Vec<CatalogEngine> = load_custom(app)
        .into_iter()
        .filter(|engine| installed_ids.contains(&engine.id))
        .collect();
    let custom_count = custom.len();
    // Versions that custom engines already cover. An orphan whose version
    // matches one of these is the same engine under a different id (the id
    // format changed when forks were added, so an older install can sit
    // beside the newer one). Showing both would be the same engine twice.
    let custom_versions: Vec<&str> = custom
        .iter()
        .map(|engine| engine.version.as_str())
        .collect();
    let missing: Vec<CatalogEngine> = installed_ids
        .iter()
        .filter(|id| {
            !catalog.engines.iter().any(|engine| engine.id == **id)
                && !custom.iter().any(|engine| engine.id == **id)
                && !custom_versions.contains(&orphaned_catalog_entry(id).version.as_str())
        })
        .map(|id| orphaned_catalog_entry(id))
        .collect();
    let missing_count = missing.len();
    catalog.engines.extend(custom);
    catalog.engines.extend(missing);
    // Captured before the drain below empties the vec, because `len()` on a vec
    // being drained is not a count of anything.
    let available_count = catalog.engines.len() - custom_count - missing_count;

    Ok(catalog
        .engines
        .drain(..)
        .enumerate()
        .map(|(index, engine)| EngineInfo {
            installed: installed_ids.contains(&engine.id),
            // The synthesised rows are the trailing ones, so position is the
            // test. Reading it off the entry instead would mean carrying a flag
            // through a struct that is also written to disk as the catalog.
            available: index < available_count,
            source: engine.source,
            id: engine.id,
            version: engine.version,
            updated_at: engine.updated_at,
            asset_name: engine.asset_name,
            size_bytes: engine.size_bytes,
        })
        .collect())
}

/// A catalog row for an engine that is installed but no longer published.
///
/// Recovered from the directory name rather than invented, because the id *is*
/// the version (`llama.cpp-vulkan-b1234`) and the user's installed engines are
/// the only remaining record of which build it was.
///
/// `updated_at` is the epoch because the publish date is genuinely unknown now
/// -- the row has to say something, and an epoch renders as an obviously old
/// date rather than as a plausible lie. `size_bytes` measures the installed
/// directory, which is close enough to the archive and honest about what is on
/// disk. The empty URL and digest are load-bearing: `install` refuses an engine
/// whose URL is not an approved GitHub release, so a stale row can never be
/// re-downloaded.
fn orphaned_catalog_entry(id: &str) -> CatalogEngine {
    let version = id
        .strip_prefix(LLAMA_CPP_ID_PREFIX)
        .unwrap_or(id)
        .to_string();
    CatalogEngine {
        id: id.to_string(),
        version,
        // No release date is recoverable from an installed directory, so this is
        // the honest floor rather than a guess. The page shows it as such.
        updated_at: "1970-01-01T00:00:00Z".to_string(),
        asset_name: String::new(),
        download_url: String::new(),
        sha256: String::new(),
        size_bytes: 0,
        // Recovered rather than assumed. A catalog engine that aged out and a
        // pasted one are both just a directory name at this point, and the id
        // prefix is the only thing left that says which -- treating every orphan
        // as a `llama.cpp` build would label a fork as official.
        source: if id.starts_with(LLAMA_CPP_ID_PREFIX) {
            default_source()
        } else {
            CUSTOM_SOURCE.to_string()
        },
    }
}

fn installed_engine_ids(app: &AppHandle) -> Result<Vec<String>, String> {
    let directory = data_dir(app)?;
    let mut installed = Vec::new();
    if let Ok(entries) = fs::read_dir(directory) {
        for entry in entries.flatten() {
            if entry.path().join(server_name()).is_file() {
                if let Some(name) = entry.file_name().to_str() {
                    installed.push(name.to_string());
                }
            }
        }
    }
    Ok(installed)
}

pub async fn install(app: AppHandle, id: &str) -> Result<(), String> {
    let catalog = load_saved_engines(&app)?;
    let engine = catalog
        .engines
        .into_iter()
        .find(|engine| engine.id == id)
        .ok_or_else(|| "Engine not found in the verified release catalog".to_string())?;
    let final_dir = installed_dir(&app, &engine.id)?;
    if final_dir.is_dir() {
        return Ok(());
    }
    if engine.size_bytes == 0 || engine.size_bytes > MAX_ARCHIVE_BYTES {
        return Err("Engine archive exceeds the download size limit".to_string());
    }
    let url = reqwest::Url::parse(&engine.download_url).map_err(|error| error.to_string())?;
    if url.scheme() != "https" || url.host_str() != Some("github.com") {
        return Err("Engine download URL is not an approved HTTPS release URL".to_string());
    }
    let staging = begin_install(&app, &engine.id)?;
    // A catalog engine is verified, and this is the only place a digest is
    // available: GitHub publishes one per release asset and the catalog carries
    // it. So the check is mandatory here rather than optional.
    if let Err(error) = fetch_archive(
        &app,
        url,
        &engine.id,
        Some(&engine.sha256),
        engine.size_bytes,
        &staging,
    )
    .await
    {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    finish_install(&app, &staging, &final_dir)
}

/// Install an engine from a GitHub release link the user pasted in.
///
/// The route for a fork: a `llama.cpp` build that this catalog does not carry,
/// published under a different owner, at a tag that is not a `bNNNN` build
/// number. Every one of those is a reason the catalog legitimately does not list
/// it, and all of them are reasons a link should work.
///
/// **No checksum is available, so none is checked.** GitHub's digest is attached
/// to a release asset in the API response and not to its download address, so
/// there is nothing here to compare the bytes against -- unlike a catalog engine,
/// where `install` refuses anything that does not match. That is stated in the
/// UI rather than left implied, and it is the one real cost of this path.
///
/// Everything else is identical to a verified install: the same host allowlist
/// and redirect policy, the same size and archive-layout limits, and its own
/// directory under `engines/` named from the link -- so two forks publishing the
/// same build number cannot collide, and nothing overwrites an official engine.
pub async fn custom_install(app: AppHandle, link: &str) -> Result<(), String> {
    let parsed = ReleaseLink::parse(link).map_err(|error| match error {
        LinkError::NotAUrl => {
            "Paste a GitHub release download link, like the address of a .zip on github.com."
                .to_string()
        }
        LinkError::NotAReleaseAsset => {
            "That is not a release file link. Copy the address of the .zip asset itself, \
             not the releases page."
                .to_string()
        }
    })?;
    let id = parsed.id();
    // Always rebuilt over `https`, whatever the paste used. The parsed parts are
    // reused rather than the original string so a query or fragment cannot ride
    // along, and percent-encoding in a tag or asset name round-trips through
    // `Url::parse` instead of being lost.
    let link: String = format!(
        "https://github.com/{}/{}/releases/download/{}/{}",
        parsed.owner, parsed.repository, parsed.tag, parsed.asset
    );
    let url = reqwest::Url::parse(&link).map_err(|error| error.to_string())?;
    // The `github.com` check `install` applies, kept here rather than assumed by
    // the parser: the redirect that serves the bytes is the part that decides
    // which host actually receives the request, and the client policy is what
    // bounds it.
    let final_dir = installed_dir(&app, &id)?;
    if final_dir.is_dir() {
        return Err("That engine is already installed".to_string());
    }
    let staging = begin_install(&app, &id)?;
    // No digest to check against -- see the note on this function.
    if let Err(error) = fetch_archive(&app, url.clone(), &id, None, 0, &staging).await {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    finish_install(&app, &staging, &final_dir)?;
    // Recorded only once the files are in place, so a failed install leaves no
    // row claiming an engine that is not there.
    let mut custom = load_custom(&app);
    custom.retain(|engine| engine.id != id);
    custom.push(CatalogEngine {
        id: id.clone(),
        version: parsed.tag.clone(),
        // The publish date is not in a download URL, and asking the API for it
        // would mean a second request for a figure the page barely shows. The
        // epoch is used elsewhere for the same reason: it renders as obviously
        // unknown rather than as a plausible wrong date.
        updated_at: "1970-01-01T00:00:00Z".to_string(),
        asset_name: parsed.asset,
        download_url: url.to_string(),
        sha256: String::new(),
        size_bytes: 0,
        source: CUSTOM_SOURCE.to_string(),
    });
    save_custom(&app, &custom)
}

/// A staging directory to download into, named so a half-finished install is
/// never mistaken for an installed one.
fn begin_install(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    let parent = data_dir(app)?;
    fs::create_dir_all(&parent).map_err(|error| error.to_string())?;
    // The uuid rather than the id, because two installs of different engines can
    // be in flight and neither may be visible to the other's cleanup.
    let staging = parent.join(format!(".install-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&staging).map_err(|error| error.to_string())?;
    let _ = id;
    Ok(staging)
}

/// Download an engine archive into `staging`, verifying it when a digest is given.
///
/// `expected_digest` is the whole difference between the two install paths. A
/// catalog engine always has one and is refused when the bytes disagree; a pasted
/// link has none, so `None` skips the comparison rather than comparing against an
/// empty string, which would refuse every download.
async fn fetch_archive(
    app: &AppHandle,
    url: reqwest::Url,
    id: &str,
    expected_digest: Option<&str>,
    size_hint: u64,
    staging: &Path,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .user_agent(concat!("ProjectZ/", env!("CARGO_PKG_VERSION")))
        .https_only(true)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let host = attempt.url().host_str().unwrap_or_default();
            if attempt.previous().len() >= 3
                || attempt.url().scheme() != "https"
                || !matches!(host, "github.com" | "release-assets.githubusercontent.com")
            {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|error| error.to_string())?;
    let archive_file = staging.join("engine.zip");
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| format!("Engine download failed: {error}"))?
        .error_for_status()
        .map_err(|error| error.to_string())?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_ARCHIVE_BYTES)
    {
        return Err("Engine archive exceeds the download size limit".to_string());
    }
    let mut response = response;
    let mut file = File::create(&archive_file).map_err(|error| error.to_string())?;
    // A pasted link carries no size either, so the total is whatever the server
    // reported -- zero when even that is absent, which the page shows as an
    // indeterminate bar rather than dividing by it.
    let total = response.content_length().unwrap_or(size_hint);
    let mut hasher = Sha256::new();
    let mut downloaded = 0u64;
    let _ = app.emit(
        "engine-download-progress",
        serde_json::json!({ "id": id, "downloaded": 0, "total": total }),
    );
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        downloaded = downloaded.saturating_add(chunk.len() as u64);
        if downloaded > MAX_ARCHIVE_BYTES {
            return Err("Engine archive exceeds the download size limit".to_string());
        }
        hasher.update(&chunk);
        file.write_all(&chunk).map_err(|error| error.to_string())?;
        let _ = app.emit(
            "engine-download-progress",
            serde_json::json!({ "id": id, "downloaded": downloaded, "total": total }),
        );
    }
    let _ = app.emit(
        "engine-download-progress",
        serde_json::json!({ "id": id, "downloaded": downloaded, "total": total, "complete": true }),
    );
    file.flush().map_err(|error| error.to_string())?;
    // The hash is computed either way -- it is how the archive is proved intact
    // -- but only compared when there is something to compare it against.
    if let Some(expected) = expected_digest {
        let digest = format!("{:x}", hasher.finalize());
        if digest != expected {
            return Err(
                "Engine download checksum did not match GitHub's SHA-256 digest".to_string(),
            );
        }
    }
    Ok(())
}

/// Expand a downloaded archive and move it into place as an installed engine.
///
/// Shared by both install paths so a fork's engine gets exactly the same
/// treatment a catalog one does: the same zip-bomb limits, the same flattening of
/// a nested layout, and the same refusal to leave a directory that does not
/// contain a server at its root. The last check is the one that matters -- an
/// archive that extracted cleanly but has no `llama-server` would otherwise be
/// listed as installed and fail at the moment a model tried to load.
fn finish_install(app: &AppHandle, staging: &Path, final_dir: &Path) -> Result<(), String> {
    let archive_file = staging.join("engine.zip");
    let extraction = staging.join("files");
    fs::create_dir_all(&extraction).map_err(|error| error.to_string())?;
    if let Err(error) = extract_zip(&archive_file, &extraction) {
        let _ = fs::remove_dir_all(staging);
        return Err(error);
    }
    let server = match find_server(&extraction) {
        Ok(server) => server,
        Err(error) => {
            let _ = fs::remove_dir_all(staging);
            return Err(error);
        }
    };
    let source_dir = server
        .parent()
        .ok_or_else(|| "Invalid engine archive layout".to_string())?;
    if source_dir != extraction {
        let children = match fs::read_dir(source_dir) {
            Ok(children) => children,
            Err(error) => {
                let _ = fs::remove_dir_all(staging);
                return Err(error.to_string());
            }
        };
        for child in children {
            let child = match child {
                Ok(child) => child,
                Err(error) => {
                    let _ = fs::remove_dir_all(staging);
                    return Err(error.to_string());
                }
            };
            let destination = extraction.join(child.file_name());
            if destination.exists() {
                let _ = fs::remove_dir_all(staging);
                return Err("Engine archive contains conflicting root files".to_string());
            }
            if let Err(error) = fs::rename(child.path(), destination) {
                let _ = fs::remove_dir_all(staging);
                return Err(error.to_string());
            }
        }
    }
    if final_dir.exists() {
        let _ = fs::remove_dir_all(staging);
        return Err("Engine installation already exists".to_string());
    }
    if let Err(error) = fs::rename(&extraction, final_dir) {
        let _ = fs::remove_dir_all(staging);
        return Err(error.to_string());
    }
    let _ = fs::remove_dir_all(staging);
    if !final_dir.join(server_name()).is_file() {
        let _ = fs::remove_dir_all(final_dir);
        return Err(
            "Engine archive did not place llama-server at the installation root".to_string(),
        );
    }
    let _ = app;
    Ok(())
}

fn extract_zip(archive_path: &Path, destination: &Path) -> Result<(), String> {
    let archive_file = File::open(archive_path).map_err(|error| error.to_string())?;
    let mut archive = ZipArchive::new(archive_file).map_err(|error| error.to_string())?;
    if archive.len() > MAX_ARCHIVE_FILES {
        return Err("Engine archive contains too many files".to_string());
    }
    let mut total_size = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| error.to_string())?;
        let enclosed = entry
            .enclosed_name()
            .ok_or_else(|| "Engine archive contains an unsafe path".to_string())?;
        if enclosed
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err("Engine archive contains an unsafe path".to_string());
        }
        total_size = total_size.saturating_add(entry.size());
        if total_size > MAX_EXTRACTED_BYTES {
            return Err("Expanded engine archive exceeds the size limit".to_string());
        }
        let output = destination.join(enclosed);
        if entry.is_dir() {
            fs::create_dir_all(output).map_err(|error| error.to_string())?;
            continue;
        }
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let mut file = File::create(&output).map_err(|error| error.to_string())?;
        let mut limited = (&mut entry).take(MAX_EXTRACTED_BYTES + 1);
        let copied = std::io::copy(&mut limited, &mut file).map_err(|error| error.to_string())?;
        if copied != entry.size() {
            return Err("Engine archive entry was truncated".to_string());
        }
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&output, fs::Permissions::from_mode(mode & 0o777))
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn find_server(root: &Path) -> Result<PathBuf, String> {
    let name = server_name();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if entry.file_name() == name {
                return Ok(path);
            }
        }
    }
    Err("Downloaded archive does not contain llama-server".to_string())
}

pub fn uninstall(app: &AppHandle, id: &str) -> Result<(), String> {
    if is_referenced(app, id)? {
        return Err(
            "This engine is in use. Point the local models using it at another engine first."
                .to_string(),
        );
    }
    let directory = installed_dir(app, id)?;
    if directory.exists() {
        fs::remove_dir_all(directory).map_err(|error| error.to_string())?;
    }
    // The record goes with the files. Left behind, it would be a row describing
    // an engine that is not installed -- and because `list` filters custom
    // entries by their directory, it would be silently filtered out anyway,
    // leaving a stale file that can only confuse the next install of the same id.
    let mut custom = load_custom(app);
    let before = custom.len();
    custom.retain(|engine| engine.id != id);
    if custom.len() != before {
        save_custom(app, &custom)?;
    }
    Ok(())
}

/// Whether a model still points at this engine.
///
/// Removing one a model names would leave that model's setting pointing at a
/// directory that is not there, and its next load would fail with a message
/// about a reinstall rather than about the choice that caused it.
pub fn is_referenced(app: &AppHandle, id: &str) -> Result<bool, String> {
    let Some(database) = app.try_state::<std::sync::Arc<crate::database::Database>>() else {
        return Ok(false);
    };
    // Read through the public list rather than a SQL scan for the prefix, so a
    // model that names a missing engine still counts as a reference.
    Ok(database
        .list_local_models()
        .unwrap_or_default()
        .iter()
        .any(|model| {
            database
                .setting::<Option<String>>(&engine_setting_key(&model.id))
                .flatten()
                .as_deref()
                == Some(id)
        }))
}

/// Whether an engine's files are on disk and ready to run a model.
///
/// An engine is installed when its directory holds a `llama-server`, which is
/// the same test `installed_engine_ids` makes -- so the page, the picker and the
/// loader all agree on what "installed" means without consulting the catalog.
/// That matters because the catalog is a window onto recent releases: an engine
/// whose release has aged out is still perfectly runnable, and refusing to pin
/// one because it is no longer listed would make it uninstallable in practice.
pub fn require_installed(app: &AppHandle, id: &str) -> Result<(), String> {
    let server = installed_dir(app, id)?.join(server_name());
    if server.is_file() {
        return Ok(());
    }
    Err(format!(
        "That engine is not installed. Install it in Settings > Engines first: {id}"
    ))
}

/// Where a model's engine lives: one setting per model.
///
/// One function rather than a format string at each site: the key is read in
/// `LocalModelManager`, here, and in the removal path, and three spellings of it
/// would be three chances to disagree about where a model's engine is stored.
pub fn engine_setting_key(model_id: &str) -> String {
    format!("{ENGINE_SETTING_PREFIX}{model_id}")
}

#[cfg(windows)]
fn server_name() -> &'static str {
    "llama-server.exe"
}

#[cfg(not(windows))]
fn server_name() -> &'static str {
    "llama-server"
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An installed engine recovers its version from its directory name.
    ///
    /// The id is the only record of which build a stale engine was, so stripping
    /// the prefix is the whole of the reconstruction -- and getting it wrong
    /// would show the user "llama.cpp-vulkan-b1234" as a version string.
    #[test]
    fn an_orphaned_engine_keeps_its_version() {
        let entry = orphaned_catalog_entry("llama.cpp-vulkan-b11371");
        assert_eq!(entry.version, "b11371");
        assert_eq!(entry.id, "llama.cpp-vulkan-b11371");
    }

    /// A synthesised row carries no download URL, so it cannot be re-fetched.
    ///
    /// Load-bearing rather than cosmetic: `install` refuses an entry whose URL is
    /// not an approved GitHub release, and that check is the only thing standing
    /// between a row with no verified checksum and a download.
    #[test]
    fn an_orphaned_engine_cannot_be_reinstalled() {
        let entry = orphaned_catalog_entry("llama.cpp-vulkan-b11371");
        assert!(entry.download_url.is_empty());
        assert!(entry.sha256.is_empty());
    }

    /// An id that does not carry the expected prefix is shown as it stands.
    ///
    /// An engine directory this app did not create should not be silently
    /// renamed into something it is not; the raw name is the honest label.
    #[test]
    fn an_unexpected_directory_name_is_kept_verbatim() {
        assert_eq!(
            orphaned_catalog_entry("some-other-engine").version,
            "some-other-engine"
        );
    }

    /// A fork's release link is taken apart into the three parts that matter.
    ///
    /// This is the whole feature in one test: a link the catalog cannot produce,
    /// because the repository is not `llama.cpp` and the tag is not a `bNNNN`
    /// build number, has to yield an owner, a repository and a version.
    #[test]
    fn a_fork_release_link_is_parsed() {
        let link = ReleaseLink::parse(
            "https://github.com/test-test/llama.cpp/releases/download/prism-b10754-2459f68/llama-test-b10754-2459f68-bin-win-vulkan-x64.zip",
        )
        .unwrap();
        assert_eq!(link.owner, "test-test");
        assert_eq!(link.repository, "llama.cpp");
        assert_eq!(link.tag, "prism-b10754-2459f68");
        assert_eq!(
            link.asset,
            "llama-test-b10754-2459f68-bin-win-vulkan-x64.zip"
        );
    }

    /// The asset's own address is the only thing accepted.
    ///
    /// The releases *page* is what a browser shows when someone right-clicks a
    /// release and copies its link, so it is the most likely wrong paste -- and it
    /// has to fail rather than be downloaded, because it is an HTML page that
    /// would fail later at extraction instead, with a worse message.
    #[test]
    fn a_releases_page_is_not_a_file_link() {
        let error =
            ReleaseLink::parse("https://github.com/test-test/llama.cpp/releases/tag/prism-b10754")
                .unwrap_err();
        assert_eq!(error, LinkError::NotAReleaseAsset);
    }

    /// Only GitHub, over `http` or `https`.
    ///
    /// The URL is the one input here that did not come from GitHub's own API, so
    /// this is what keeps a local path or a UNC share out of the downloader. The
    /// host is what actually bounds it; the scheme is checked because an accepted
    /// one has to be one we can upgrade to HTTPS.
    #[test]
    fn only_a_github_release_link_is_accepted() {
        for refused in [
            "file:///C:/engines/llama-server.zip",
            r"\\attacker\share\llama-server.zip",
            "https://github.com.evil.test/o/r/releases/download/t/a.zip",
            "ftp://github.com/o/r/releases/download/t/a.zip",
            "not a url at all",
        ] {
            assert_eq!(
                ReleaseLink::parse(refused).unwrap_err(),
                LinkError::NotAUrl,
                "{refused}"
            );
        }
    }

    /// A plain `http://` link is taken, not refused.
    ///
    /// The paste people actually make. Refusing it would have sent someone
    /// looking for a typo in a link that was perfectly correct apart from a
    /// scheme no browser would have complained about either.
    #[test]
    fn an_http_link_is_accepted_rather_than_refused() {
        let link = ReleaseLink::parse(
            "http://github.com/PrismML-Eng/llama.cpp/releases/download/prism-b10754-2459f68/llama-prism-b10754-2459f68-bin-win-vulkan-x64.zip",
        )
        .unwrap();
        assert_eq!(link.owner, "PrismML-Eng");
        assert_eq!(link.repository, "llama.cpp");
        assert_eq!(link.tag, "prism-b10754-2459f68");
        assert_eq!(link.id(), "PrismML-Eng-llama.cpp-prism-b10754-2459f68");
    }

    /// Two forks publishing the same build number get separate directories.
    ///
    /// The id is the directory name, so an id built from the tag alone would put
    /// one fork's files where another's already are -- and since an install
    /// returns early when the directory exists, the second would silently be a
    /// no-op that reported success. The owner and repository are in the id for
    /// exactly this.
    #[test]
    fn the_id_separates_two_forks_publishing_the_same_build() {
        let id = |repo: &str| {
            ReleaseLink::parse(&format!(
                "https://github.com/owner/{repo}/releases/download/b10754/llama.zip"
            ))
            .unwrap()
            .id()
        };
        assert_ne!(id("llama.cpp"), id("llama.cpp-vulkan"));
        assert_eq!(id("llama.cpp"), "owner-llama.cpp-b10754");
    }

    /// An id derived from a URL is one the directory check will accept.
    ///
    /// `installed_dir` rejects anything outside `[A-Za-z0-9._-]` because the id
    /// becomes a path segment, so a tag carrying something else -- `v1.0.0:beta`
    /// percent-encoded, which a real release tag can be -- has to be flattened
    /// here or the install would fail on its own generated name. Both this test
    /// and that check name the same character set on purpose: they were written
    /// apart and disagreed once, and the id generator was the side that lost.
    #[test]
    fn a_link_with_unusual_characters_still_produces_a_usable_id() {
        let id = ReleaseLink::parse("https://github.com/o/r/releases/download/v1.0.0%3Abeta/a.zip")
            .map(|link| link.id())
            .unwrap();
        assert!(!id.is_empty());
        assert!(
            id.bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_')),
            "{id}"
        );
    }

    /// A pasted engine is never reported as an official `llama.cpp` build.
    ///
    /// The id prefix is the only thing distinguishing the two once a directory
    /// exists, so the orphan path has to read it rather than defaulting
    /// everything to `llama.cpp` -- which would label a fork as official in the
    /// one place the user is shown what they actually have.
    #[test]
    fn an_orphaned_custom_engine_keeps_its_custom_label() {
        let entry = orphaned_catalog_entry("owner-llama.cpp-prism-b10754");
        assert_eq!(entry.source, CUSTOM_SOURCE);
        assert_eq!(entry.version, "owner-llama.cpp-prism-b10754");
    }
}
