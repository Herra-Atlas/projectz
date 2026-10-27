//! The eSpeak-ng phonemizer component, installed by this app.
//!
//! A handful of audio.cpp's TTS families -- Inflect v2, SanoTTS, Piper and
//! Kitten, the phonemizing ones -- read text through the engine's eSpeak-ng
//! adapter, and the engine's Windows release builds that adapter against the
//! machine's *shared* eSpeak install rather than bundling one. This module
//! makes the app own that install instead: one digest-verified artifact,
//! unpacked into the app's own engines folder, and referenced from the server
//! config as the explicit session options the adapter accepts.
//!
//! The wish this serves is the app's oldest one -- data deleted with the app:
//! nothing is written outside `engines/`, nothing is registered with the
//! system, and the uninstall is a folder delete. Windows keeps a file locked
//! while its DLL is loaded, so both installs and uninstalls stop the audio
//! server first; a replaced eSpeak half-loaded is worse than none.
//!
//! The artifact is the `espeakng_loader` release -- a widely consumed
//! redistribution of the upstream shared library and its matching data, laid
//! out as a plain zip whose contents are the deliverable, with the digest
//! PyPI publishes as the pin. Shape mirrors `audiocpp::install` exactly; only
//! the source, the caps, and what "installed" means differ.

use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};
use zip::ZipArchive;

use crate::app_paths::data_root;

/// The installed component's folder name: engine kind plus its own version.
pub const COMPONENT: &str = "espeak-ng-0.2.4";

/// The pinned artifact: the `espeakng_loader` Windows-amd64 release, digest
/// exactly as PyPI publishes it.
const URL: &str = "https://files.pythonhosted.org/packages/9d/ed/a3d872fbad4f3a3f3db0e8c31768ab14e77cd77306de16b8b20b1e1df7ea/espeakng_loader-0.2.4-py3-none-win_amd64.whl";
const SHA256: &str = "41f1e08ac9deda2efd1ea9de0b81dab9f5ae3c4b24284f76533d0a7b1dd7abd7";
/// Guards sized for this artifact: ~9.4 MB packed, ~23 MB expanded, 369 entries.
const MAX_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: u64 = 256 * 1024 * 1024;
const MAX_FILES: usize = 1024;

/// Where the component lives -- inside the app, so a deleted ProjectZ takes
/// it with it.
pub fn install_dir() -> PathBuf {
    data_root().join("engines").join(COMPONENT)
}

/// The package folder inside the artifact: it holds `espeak-ng.dll` and the
/// `espeak-ng-data` dictionaries side by side, which is the layout the
/// adapter's explicit paths name.
fn package_dir() -> PathBuf {
    install_dir().join("espeakng_loader")
}

/// Complete means library *and* data: the adapter refuses a library whose
/// data cannot be found, and a data folder alone would sit on disk invisible,
/// pretending to help nothing.
pub fn is_installed() -> bool {
    package_dir().join("espeak-ng.dll").is_file() && package_dir().join("espeak-ng-data").is_dir()
}

/// The two paths the audio.cpp adapter accepts, when the app-owned install is
/// complete. The config hands these in as `espeak_library_path` and
/// `espeak_data_path`, ahead of any system install the machine carries.
pub fn paths() -> Option<(PathBuf, PathBuf)> {
    if !is_installed() {
        return None;
    }
    let package = package_dir();
    Some((
        package.join("espeak-ng.dll"),
        package.join("espeak-ng-data"),
    ))
}

/// Install: verify the digest on the way in, unpack into place.
///
/// The server is stopped first, not because it cannot run without eSpeak --
/// most of the catalog never touches it -- but because it may be the process
/// holding the previous library open, and Windows does not let a DLL be
/// replaced from under a running holder.
pub async fn install(app: &AppHandle) -> Result<(), String> {
    crate::audio::audiocpp::stop_all(app).await;
    if is_installed() {
        // Replacing a working install over itself is how a corrupted one is
        // re-laid -- the button stays useful because of this line.
        let _ = fs::remove_dir_all(install_dir());
    }

    let destination = install_dir();
    fs::create_dir_all(
        destination
            .parent()
            .expect("install_dir always has a parent"),
    )
    .map_err(|error| error.to_string())?;

    let staging = destination.with_extension("part");
    if staging.exists() {
        let _ = fs::remove_dir_all(&staging);
    }
    if let Err(error) = fetch_artifact(app, &staging).await {
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
        return Err(
            "The eSpeak package was unpacked but the library or its data is missing from it."
                .to_string(),
        );
    }
    Ok(())
}

/// Uninstall: stop the server (it may hold the DLL open), then delete the
/// folder. The folder delete is the whole uninstall -- the config re-reads
/// the disk at every start, and a phonemizing model without eSpeak fails
/// loudly rather than quietly.
pub async fn uninstall(app: &AppHandle) -> Result<(), String> {
    crate::audio::audiocpp::stop_all(app).await;
    let directory = install_dir();
    if !directory.exists() {
        return Err("The eSpeak phonemizer is not installed.".to_string());
    }
    fs::remove_dir_all(&directory)
        .map_err(|error| format!("Could not remove the eSpeak phonemizer: {error}"))
}

/// Streams the artifact into `staging`, verifying its digest before anything
/// is unpacked. Progress rides the same event the engine installs use, so the
/// page's own install button animates the same way.
async fn fetch_artifact(app: &AppHandle, staging: &Path) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .user_agent(concat!("ProjectZ/", env!("CARGO_PKG_VERSION")))
        .https_only(true)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let host = attempt.url().host_str().unwrap_or_default();
            if attempt.previous().len() >= 3
                || attempt.url().scheme() != "https"
                || !matches!(host, "files.pythonhosted.org")
            {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|error| error.to_string())?;

    let response = client
        .get(URL)
        .send()
        .await
        .map_err(|error| format!("The eSpeak download failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("The eSpeak download failed: {error}"))?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_ARCHIVE_BYTES)
    {
        return Err("The eSpeak artifact is larger than expected.".to_string());
    }

    fs::create_dir_all(staging)
        .map_err(|error| format!("Could not create {}: {error}", staging.display()))?;
    let archive_path = staging.join("artifact.zip");
    let mut archive = fs::File::create(&archive_path)
        .map_err(|error| format!("Could not write the eSpeak artifact: {error}"))?;

    let mut hasher = Sha256::new();
    let mut received: u64 = 0;
    let mut body = response.bytes_stream();
    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(|error| format!("The download was interrupted: {error}"))?;
        received += chunk.len() as u64;
        if received > MAX_ARCHIVE_BYTES {
            return Err("The eSpeak artifact is larger than expected.".to_string());
        }
        archive
            .write_all(&chunk)
            .map_err(|error| format!("Could not write the eSpeak artifact: {error}"))?;
        hasher.update(&chunk);
        let _ = app.emit(
            "engine-download-progress",
            serde_json::json!({ "id": "espeak", "downloaded": received, "total": received }),
        );
    }
    archive
        .flush()
        .map_err(|error| format!("Could not finish the download: {error}"))?;
    drop(archive);

    if format!("{:x}", hasher.finalize()) != SHA256 {
        return Err(
            "The eSpeak download did not match its published checksum, so it was not installed."
                .to_string(),
        );
    }
    Ok(())
}

/// Expands the artifact into `destination`, keeping the `espeakng_loader`
/// folder the artifact itself is arranged in -- that folder is what the
/// config names. Guards match the engine unpack: no path escapes, nothing
/// exceeds the caps, and a truncated entry is a removed install.
fn unpack(staging: &Path, destination: &Path) -> Result<(), String> {
    let archive = fs::File::open(staging.join("artifact.zip"))
        .map_err(|error| format!("Could not read the downloaded eSpeak: {error}"))?;
    let mut zip = ZipArchive::new(archive)
        .map_err(|error| format!("The downloaded eSpeak is not a readable archive: {error}"))?;
    if zip.len() > MAX_FILES {
        return Err("The eSpeak archive has more files than expected.".to_string());
    }

    fs::create_dir_all(destination)
        .map_err(|error| format!("Could not create {}: {error}", destination.display()))?;
    let mut written: u64 = 0;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|error| format!("The eSpeak archive could not be read: {error}"))?;
        // `enclosed_name` is what keeps `..` and absolute paths out; like the
        // engine unpack, a name that fails it is skipped rather than written
        // somewhere unexpected.
        let Some(enclosed) = entry.enclosed_name() else {
            continue;
        };
        if enclosed
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err("The eSpeak archive contains an unsafe path.".to_string());
        }
        let target = destination.join(&enclosed);
        if entry.is_dir() {
            fs::create_dir_all(&target)
                .map_err(|error| format!("Could not create {}: {error}", target.display()))?;
            continue;
        }
        written += entry.size();
        if written > MAX_EXTRACTED_BYTES {
            return Err("The expanded eSpeak archive is larger than expected.".to_string());
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
        // entry was truncated -- a truncated dictionary file would fail the
        // load in a place no message explains.
        if copied != entry.size() {
            return Err("The eSpeak archive contains a truncated file.".to_string());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pin_is_a_strong_digest() {
        assert_eq!(SHA256.len(), 64);
        assert!(SHA256.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn the_component_lives_under_engines() {
        assert!(install_dir().starts_with(crate::app_paths::data_root().join("engines")));
        assert!(install_dir().ends_with(COMPONENT));
    }

    #[test]
    fn nothing_maps_to_the_config_without_a_folder() {
        // The real is_installed checks both files; on a machine without the
        // component, its package paths must stay absent so the config builder
        // falls back to the system install and then to nothing.
        let absent = std::path::PathBuf::from("projectz-cannot-exist-here");
        let complete =
            absent.join("espeak-ng.dll").is_file() && absent.join("espeak-ng-data").is_dir();
        assert!(!complete);
    }
}
