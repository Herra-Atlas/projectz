//! The Piper engine, installed from its pinned release.
//!
//! # Why a download rather than bundled
//!
//! `voice/` was "placed by the build" until the pages wanted installs and
//! uninstalls, which means it had to become a fact on disk the user controls
//! like every other engine. Piper's releases carry no per-asset digest, so the
//! verification is the whisper model again: one tag, one asset, one SHA-256
//! written here and never taken from "whatever is latest".
//!
//! # Why the constants are checked together
//!
//! A new build replaces all five values as one edit, so a release that changed
//! its packaging (a different root folder, a second exe) cannot fail only at
//! the hash check while claiming the layout above.

use std::io::Write;
use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};
use zip::ZipArchive;

/// The release this app installs, and the name it shows.
pub const RELEASE_TAG: &str = "2023.11.14-2";
/// The binary the TTS path runs.
pub const PIPER_EXE: &str = "piper.exe";

const ASSET: &str = "piper_windows_amd64.zip";
const ASSET_URL: &str =
    "https://github.com/rhasspy/piper/releases/download/2023.11.14-2/piper_windows_amd64.zip";
const ASSET_SHA256: &str = "f3c58906402b24f3a96d92145f58acba6d86c9b5db896d207f78dc80811efcea";
/// The packed size, used only to refuse an archive that is obviously not this one.
const ASSET_BYTES: u64 = 22_477_236;

/// Guards on unpacking: the real archive is 363 entries and 39 MB unpacked; a
/// replaced asset cannot fill a disk through this path.
const MAX_EXTRACTED_BYTES: u64 = 256 * 1024 * 1024;
const MAX_FILES: usize = 1024;

/// Where the engine lives. `voice/piper` is the root the synthesis path already
/// names, so an install places it exactly where `piper::resolve_voice` looks.
pub fn install_dir() -> PathBuf {
    crate::app_paths::voice_root().join("piper")
}

/// Whether the install is complete enough to speak.
pub fn is_installed() -> bool {
    install_dir().join(PIPER_EXE).is_file()
}

/// Downloads, verifies and unpacks the engine.
///
/// Returns at once when it is already installed, so the button can be pressed
/// twice without costing a second download.
pub async fn install(app: &AppHandle) -> Result<(), String> {
    if is_installed() {
        return Ok(());
    }

    let destination = install_dir();
    // A directory left by an interrupted attempt cannot be trusted -- the exe may
    // be present while a DLL it loads is not -- so it is replaced whole.
    if destination.exists() {
        std::fs::remove_dir_all(&destination)
            .map_err(|error| format!("Could not clear {}: {error}", destination.display()))?;
    }
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
    }

    let staging = destination.with_extension("part");
    if let Err(error) = fetch(app, &staging).await {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(error);
    }
    let unpack_to = staging.join("files");
    if let Err(error) = unpack(&staging, &unpack_to) {
        let _ = std::fs::remove_dir_all(&destination);
        return Err(error);
    }
    // The archive's own shape is a root folder (`piper/`), and the install root
    // is named already: the folder inside the archive is what moves in.
    let source = find_root(&unpack_to)?;
    for entry in std::fs::read_dir(&source)
        .map_err(|error| format!("Could not read the unpacked engine: {error}"))?
    {
        let entry =
            entry.map_err(|error| format!("Could not read the unpacked engine: {error}"))?;
        std::fs::rename(entry.path(), destination.join(entry.file_name()))
            .map_err(|error| format!("Could not place {}: {error}", entry.path().display()))?;
    }
    let _ = std::fs::remove_dir_all(&staging);

    if !is_installed() {
        let _ = std::fs::remove_dir_all(&destination);
        return Err(format!(
            "The engine was unpacked but {PIPER_EXE} is not in it."
        ));
    }
    Ok(())
}

/// Removes the engine, refusing while voices can still use it.
///
/// A voice for which the runtime is gone fails at speak time with a message
/// about an engine rather than about the delete that caused it, so the delete
/// is what refuses first.
pub fn uninstall() -> Result<(), String> {
    // Only Piper voices tell the engine apart from audio.cpp ones: an audio.cpp
    // voice does not need this folder, and its files are not a reason to refuse.
    if crate::audio::tts::voices::piper_voices_installed() {
        return Err(
            "Piper voices are installed. Delete them from the Text to speech page first."
                .to_string(),
        );
    }
    let directory = install_dir();
    if !directory.exists() {
        return Err("Piper is not installed.".to_string());
    }
    std::fs::remove_dir_all(&directory)
        .map_err(|error| format!("Piper could not be removed: {error}"))
}

/// Streams the archive into `staging`, hashing as it goes.
///
/// The hash is checked against the pinned digest before anything is unpacked:
/// a truncated download is the common failure, and a `.zip` that opens only
/// halfway is worse than one that never opened.
async fn fetch(app: &AppHandle, staging: &Path) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("projectz/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| format!("Could not prepare the download: {error}"))?;

    let response = client
        .get(ASSET_URL)
        .send()
        .await
        .map_err(|error| format!("Could not download the voice engine: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Could not download the voice engine: GitHub answered {}",
            response.status().as_u16()
        ));
    }

    let total = response.content_length();
    std::fs::create_dir_all(staging)
        .map_err(|error| format!("Could not create {}: {error}", staging.display()))?;
    let archive_path = staging.join(ASSET);
    let mut archive = std::fs::File::create(&archive_path)
        .map_err(|error| format!("Could not write {}: {error}", archive_path.display()))?;

    let mut hasher = Sha256::new();
    let mut received: u64 = 0;
    let mut body = response.bytes_stream();
    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(|error| format!("The download was interrupted: {error}"))?;
        archive
            .write_all(&chunk)
            .map_err(|error| format!("Could not write the engine: {error}"))?;
        hasher.update(&chunk);
        received += chunk.len() as u64;
        let _ = app.emit(
            "audio-engine-progress",
            serde_json::json!({ "id": "piper", "downloaded": received, "total": total }),
        );
    }
    archive
        .flush()
        .map_err(|error| format!("Could not finish the download: {error}"))?;
    drop(archive);

    // A size that is not even close means this is not the file the constants
    // describe -- a captive portal page, say -- and the digest below would report
    // that as a mismatch without saying why.
    if received < ASSET_BYTES / 2 {
        return Err(format!(
            "The voice engine download looks incomplete ({received} bytes)."
        ));
    }

    let digest = format!("{:x}", hasher.finalize());
    if digest != ASSET_SHA256 {
        return Err(
            "The voice engine did not match its published checksum, so it was not installed."
                .to_string(),
        );
    }
    Ok(())
}

/// Unpacks the archive beside its final home, refusing anything that escapes it.
fn unpack(staging: &Path, destination: &Path) -> Result<(), String> {
    let archive = std::fs::File::open(staging.join(ASSET))
        .map_err(|error| format!("Could not read the downloaded engine: {error}"))?;
    let mut zip = ZipArchive::new(archive)
        .map_err(|error| format!("The downloaded engine is not a readable archive: {error}"))?;
    if zip.len() > MAX_FILES {
        return Err("The voice engine archive has more files than expected.".to_string());
    }

    std::fs::create_dir_all(destination)
        .map_err(|error| format!("Could not create {}: {error}", destination.display()))?;

    let mut written: u64 = 0;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|error| format!("The engine archive could not be read: {error}"))?;
        // `enclosed_name` is what keeps `..` and absolute paths out; an entry that
        // fails it is skipped rather than written somewhere unexpected.
        let Some(relative) = entry.enclosed_name() else {
            continue;
        };
        let target = destination.join(&relative);

        if entry.is_dir() {
            std::fs::create_dir_all(&target)
                .map_err(|error| format!("Could not create {}: {error}", target.display()))?;
            continue;
        }
        written += entry.size();
        if written > MAX_EXTRACTED_BYTES {
            return Err("The voice engine archive is larger than expected.".to_string());
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
        }
        let mut file = std::fs::File::create(&target)
            .map_err(|error| format!("Could not write {}: {error}", target.display()))?;
        std::io::copy(&mut entry, &mut file)
            .map_err(|error| format!("Could not unpack {}: {error}", target.display()))?;
    }
    Ok(())
}

/// The single top-level folder the archive carries, which is where its files
/// sit for the move into the install root.
fn find_root(unpacked: &Path) -> Result<PathBuf, String> {
    let entries = std::fs::read_dir(unpacked)
        .map_err(|error| format!("Could not read the unpacked engine: {error}"))?;
    for entry in entries.flatten() {
        if entry.path().is_dir() {
            return Ok(entry.path());
        }
    }
    Ok(unpacked.to_path_buf())
}
