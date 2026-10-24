//! The audio.cpp engine, installed from its GitHub release catalog.
//!
//! # Why a live catalog rather than pinned constants
//!
//! audio.cpp publishes a build wave per release, and a pinned constant would be
//! a date in this source tree the user could not inspect: every asset of every
//! release comes back from GitHub's own release API with the digest published
//! beside it, so the catalog is dynamic and the verification is still real. The
//! llama.cpp engines have worked this way for a while, and the install path
//! below is deliberately the same shape -- staged download, digest check, size
//! and file-count guards, and a directory named after the release.
//!
//! # Which asset
//!
//! `audio-<tag>-bin-windows-x64-vulkan.zip`: the desktop build that needs no
//! CUDA runtime beside it, on the same backend the whisper.cpp engine already
//! runs on. A `-portable` asset is deliberately not matched -- it is for
//! machines without a package manager, and two assets for one install would be
//! two answers to the same question.

use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};
use zip::ZipArchive;

use super::{engines_root, valid_id};

const RELEASES_URL: &str = "https://api.github.com/repos/0xShug0/audio.cpp/releases?per_page=30";

/// The id prefix, which is also how a directory on disk is recognised as this
/// engine's install.
pub const ID_PREFIX: &str = "audiocpp-";

/// The suffix the desktop Vulkan build carries, one asset per release.
const ASSET_SUFFIX: &str = "-bin-windows-x64-vulkan.zip";
const ASSET_PREFIX: &str = "audio-";

/// How long the cached catalog answers for.
const CACHE_AGE: Duration = Duration::from_secs(60 * 60 * 12);

/// Guards carried over from the llama.cpp path, sized for this archive: ~63 MB
/// packed, ~184 MB unpacked across ~260 entries -- the `model_specs/` folder the
/// release ships alone carries dozens of spec files. The count is padded well
/// beyond that so a repacked layout still fits, while staying far under what an
/// archive expanded into thousands of junk entries would be.
const MAX_ARCHIVE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_FILES: usize = 1024;

/// What one catalog row tells the page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalog {
    pub fetched_at: u64,
    pub engines: Vec<Entry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// `audiocpp-<tag>`, which is also the directory installed into.
    pub id: String,
    pub version: String,
    pub updated_at: String,
    pub asset_name: String,
    pub download_url: String,
    /// GitHub's own digest for the asset, `sha256:`-stripped.
    pub sha256: String,
    pub size_bytes: u64,
}

/// One release as the API answers it.
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    published_at: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<ReleaseAsset>,
}

#[derive(Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

/// Where this engine's catalog is cached: a file beside the llama.cpp one,
/// under the engines root rather than app-data, because that is where an
/// engine's own state already lives.
fn cache_path() -> PathBuf {
    engines_root().join("audiocpp-catalog.json")
}

/// Where `id` installs to.
fn install_dir_for(id: &str) -> Result<PathBuf, String> {
    if !valid_id(id) {
        return Err("Invalid engine identifier".to_string());
    }
    Ok(engines_root().join(id))
}

/// Where the currently installed engine sits, when one is.
pub fn installed_dir() -> Option<PathBuf> {
    let Ok(entries) = fs::read_dir(engines_root()) else {
        return None;
    };
    for entry in entries.flatten() {
        if let Some(id) = entry.file_name().to_str() {
            if id.starts_with(ID_PREFIX) && installed_ids().contains(&id.to_string()) {
                return Some(entry.path());
            }
        }
    }
    None
}

/// Every directory under the engines root that holds this engine's server.
pub fn installed_ids() -> Vec<String> {
    let mut ids = Vec::new();
    let Ok(entries) = fs::read_dir(engines_root()) else {
        return ids;
    };
    for entry in entries.flatten() {
        let is_engine = entry.path().join(server_name()).is_file();
        if !is_engine {
            continue;
        }
        if let Some(name) = entry.file_name().to_str() {
            if name.starts_with(ID_PREFIX) {
                ids.push(name.to_string());
            }
        }
    }
    ids
}

/// The server binary, whatever the platform spells it.
fn server_name() -> &'static str {
    if cfg!(windows) {
        "audiocpp_server.exe"
    } else {
        "audiocpp_server"
    }
}

/// The installed engine's server path, when it is installed at all.
pub fn server_executable() -> Option<PathBuf> {
    installed_dir().map(|dir| dir.join(server_name()))
}

/// Whether the engine's files are on disk.
pub fn is_installed() -> bool {
    server_executable().is_some()
}

/// The tag the installed engine carries, from its directory name.
pub fn installed_version() -> Option<String> {
    installed_dir().and_then(|dir| {
        dir.file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.trim_start_matches(ID_PREFIX).to_string())
    })
}

/// The catalog, cached for `CACHE_AGE`, refreshed on ask.
pub async fn list(refresh: bool) -> Result<Vec<Entry>, String> {
    let cache = fs::read(cache_path()).ok().and_then(|bytes| {
        let catalog: Catalog = serde_json::from_slice(&bytes).ok()?;
        let fresh = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            - catalog.fetched_at
            < CACHE_AGE.as_secs();
        fresh.then_some(catalog)
    });
    if let (false, Some(catalog)) = (refresh, cache) {
        return Ok(catalog.engines);
    }
    let fresh = fetch().await?;
    let _ = fs::write(cache_path(), serde_json::to_vec(&fresh).unwrap_or_default());
    Ok(fresh.engines)
}

/// The catalog, read from GitHub's release API.
///
/// One asset per release is taken -- the desktop Vulkan build -- and a release
/// without one, or without a digest for it, is skipped: verification is part of
/// what a catalog row *is* here, so a row without a digest would be an engine
/// the installed path could never check.
async fn fetch() -> Result<Catalog, String> {
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
        .map_err(|error| format!("Could not read the audio.cpp releases: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Could not read the audio.cpp releases: {error}"))?
        .json()
        .await
        .map_err(|error| format!("Could not read the audio.cpp releases: {error}"))?;

    let engines = releases
        .into_iter()
        .filter(|release| !release.draft && !release.prerelease)
        .filter_map(|release| {
            let asset = release.assets.into_iter().find(|asset| {
                asset.name.starts_with(ASSET_PREFIX) && asset.name.ends_with(ASSET_SUFFIX)
            })?;
            let digest = asset
                .digest
                .as_deref()
                .and_then(|value| value.strip_prefix("sha256:"))?;
            Some(Entry {
                id: format!("{ID_PREFIX}{}", release.tag_name),
                version: release.tag_name,
                updated_at: release.published_at,
                asset_name: asset.name,
                download_url: asset.browser_download_url,
                sha256: digest.to_ascii_lowercase(),
                size_bytes: asset.size,
            })
        })
        .take(10)
        .collect();
    Ok(Catalog {
        fetched_at: now_unix(),
        engines,
    })
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Installs one catalog entry, verifying its digest on the way in.
///
/// Returns at once when the id is already installed, so a second press of a
/// download button is free.
pub async fn install(app: &AppHandle, id: &str) -> Result<(), String> {
    let entry = list(false)
        .await?
        .into_iter()
        .find(|entry| entry.id == id)
        .ok_or_else(|| "That engine is not in the published release catalog.".to_string())?;
    let destination = install_dir_for(&entry.id)?;
    if destination.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(engines_root())
        .map_err(|error| format!("Could not create {}: {error}", engines_root().display()))?;

    let staging = destination.with_extension("part");
    if staging.exists() {
        let _ = fs::remove_dir_all(&staging);
    }
    if let Err(error) = fetch_archive(app, &entry, &staging).await {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    if let Err(error) = unpack(&staging, &destination) {
        let _ = fs::remove_dir_all(&destination);
        return Err(error);
    }
    let _ = fs::remove_dir_all(&staging);

    if !is_installed() {
        let _ = fs::remove_dir_all(&destination);
        return Err(format!(
            "The engine was unpacked but {server_name} is not in it.",
            server_name = server_name()
        ));
    }
    Ok(())
}

/// Streams the archive into `staging`, verifying the digest before anything is
/// unpacked.
async fn fetch_archive(app: &AppHandle, entry: &Entry, staging: &Path) -> Result<(), String> {
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

    let response = client
        .get(&entry.download_url)
        .send()
        .await
        .map_err(|error| format!("The engine download failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("The engine download failed: {error}"))?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_ARCHIVE_BYTES)
    {
        return Err("The engine archive is larger than expected.".to_string());
    }

    fs::create_dir_all(staging)
        .map_err(|error| format!("Could not create {}: {error}", staging.display()))?;
    let archive_path = staging.join("engine.zip");
    let mut archive = fs::File::create(&archive_path).map_err(|error| format!("{error}"))?;

    let mut hasher = Sha256::new();
    let mut received: u64 = 0;
    let mut body = response.bytes_stream();
    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(|error| format!("The download was interrupted: {error}"))?;
        received += chunk.len() as u64;
        if received > MAX_ARCHIVE_BYTES {
            return Err("The engine archive is larger than expected.".to_string());
        }
        archive
            .write_all(&chunk)
            .map_err(|error| format!("Could not write the engine: {error}"))?;
        hasher.update(&chunk);
        let _ = app.emit(
            "engine-download-progress",
            serde_json::json!({ "id": entry.id, "downloaded": received, "total": entry.size_bytes }),
        );
    }
    archive
        .flush()
        .map_err(|error| format!("Could not finish the download: {error}"))?;
    drop(archive);

    let digest = format!("{:x}", hasher.finalize());
    if digest != entry.sha256 {
        return Err(
            "The engine did not match its published checksum, so it was not installed.".to_string(),
        );
    }
    Ok(())
}

/// Unpacks the archive, refusing anything that escapes its folder or is larger
/// than the guards above.
fn unpack(staging: &Path, destination: &Path) -> Result<(), String> {
    let archive = fs::File::open(staging.join("engine.zip"))
        .map_err(|error| format!("Could not read the downloaded engine: {error}"))?;
    let mut zip = ZipArchive::new(archive)
        .map_err(|error| format!("The downloaded engine is not a readable archive: {error}"))?;
    if zip.len() > MAX_FILES {
        return Err("The engine archive has more files than expected.".to_string());
    }

    fs::create_dir_all(destination)
        .map_err(|error| format!("Could not create {}: {error}", destination.display()))?;
    let mut written: u64 = 0;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|error| format!("The engine archive could not be read: {error}"))?;
        // `enclosed_name` is what keeps `..` and absolute paths out; one that
        // fails it is skipped rather than written somewhere unexpected.
        let Some(relative) = entry.enclosed_name() else {
            continue;
        };
        if relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err("The engine archive contains an unsafe path.".to_string());
        }
        let target = destination.join(&relative);
        if entry.is_dir() {
            fs::create_dir_all(&target)
                .map_err(|error| format!("Could not create {}: {error}", target.display()))?;
            continue;
        }
        written += entry.size();
        if written > MAX_EXTRACTED_BYTES {
            return Err("The expanded engine archive is larger than expected.".to_string());
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
        }
        let mut file = fs::File::create(&target)
            .map_err(|error| format!("Could not write {}: {error}", target.display()))?;
        let mut taken = (&mut entry).take(MAX_EXTRACTED_BYTES);
        let copied = std::io::copy(&mut taken, &mut file)
            .map_err(|error| format!("Could not unpack {}: {error}", target.display()))?;

        // The byte count the archive states is what landed on disk, or the
        // entry was truncated -- and a truncated binary is worse than a failed
        // install, so both are removed.
        if copied != entry.size() {
            return Err("The engine archive contains a truncated file.".to_string());
        }
    }
    Ok(())
}

/// Removes one engine install, refusing while the server may be running.
pub fn uninstall(id: &str) -> Result<(), String> {
    let directory = install_dir_for(id)?;
    if !directory.exists() {
        return Err("That engine is not installed.".to_string());
    }
    // The server is a process this app owns; asking it to keep its files while
    // a child may still hold a handle would leave a half-removed engine behind.
    // A running server is stopped from the Audio page's own switch.
    fs::remove_dir_all(directory).map_err(|error| format!("Could not remove the engine: {error}"))
}
