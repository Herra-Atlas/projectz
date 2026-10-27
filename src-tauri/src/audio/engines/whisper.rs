//! The whisper.cpp engine, installed from this project's own build release.
//!
//! # Why a download rather than a bundled binary
//!
//! 16 MB packed, 47 MB unpacked, and most of it is the Vulkan backend. Bundling
//! it would put that in every install, including the machines that never turn
//! speech on -- so the Local page offers it instead, exactly as it offers a
//! llama.cpp engine.
//!
//! # Why the tag, name and digest are constants
//!
//! This is a verified install: one release, one asset, one SHA-256. A new build
//! replaces these four values together; nothing trusts "whatever the latest
//! release happens to be".

use std::io::Write;
use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};
use zip::ZipArchive;

use crate::app_paths::data_root;

/// The release this app installs, and the name it shows.
pub const RELEASE_TAG: &str = "whisper-vulkan-b5454";

/// What the engine can be asked to run. It comes from the same archive as
/// `parakeet-cli.exe`, which the transcription step will use for Parakeet models.
pub const WHISPER_EXE: &str = "whisper-cli.exe";

/// Parakeet models are read by their own executable, shipped in the same archive.
pub const PARAKEET_EXE: &str = "parakeet-cli.exe";

/// The same engine, running as a server instead of one file per press.
///
/// A different executable, not a flag: passing server arguments to `whisper-cli`
/// makes it print its help and exit successfully, which looks exactly like a
/// server that never opened its port.
pub const SERVER_EXE: &str = "whisper-server.exe";

const ASSET: &str = "whisper-vulkan-b5454.zip";
const ASSET_URL: &str = "https://github.com/Herra-Atlas/vulkan-builds/releases/download/whisper-vulkan-b5454/whisper-vulkan-b5454.zip";
const ASSET_SHA256: &str = "ee21a1617bbc54a325c0b3978f9abeadf9c521f88c7325c86ee56ea32cfe4f41";
/// The packed size, used only to refuse an archive that is obviously not this one.
const ASSET_BYTES: u64 = 16_105_093;

/// Guards on unpacking. The real archive holds a dozen files and 47 MB; these
/// exist so a replaced asset cannot fill the disk.
const MAX_EXTRACTED_BYTES: u64 = 256 * 1024 * 1024;
const MAX_FILES: usize = 256;

/// Where the engine lives, named after the release -- the same shape the
/// llama.cpp engines use, so a second build lands beside this one rather than
/// overwriting it.
pub fn install_dir() -> PathBuf {
    data_root().join("engines").join(RELEASE_TAG)
}

/// One of the engine's executables, if the install is complete.
pub fn executable(name: &str) -> Option<PathBuf> {
    let path = install_dir().join(name);
    path.is_file().then_some(path)
}

/// Whether the engine is unpacked and runnable.
pub fn is_installed() -> bool {
    executable(WHISPER_EXE).is_some()
}

/// What the installing row is told while the archive arrives.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub received: u64,
    pub total: Option<u64>,
}

/// Downloads and unpacks the engine.
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
    if let Err(error) = unpack(&staging, &destination) {
        let _ = std::fs::remove_dir_all(&destination);
        return Err(error);
    }
    // The unpacked copy is what matters; the archive has done its job.
    let _ = std::fs::remove_dir_all(&staging);

    if !is_installed() {
        return Err(format!(
            "The engine was unpacked but {WHISPER_EXE} is not in it."
        ));
    }
    Ok(())
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
        .map_err(|error| format!("Could not download the speech engine: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Could not download the speech engine: GitHub answered {}",
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
        let _ = app.emit("stt-progress", Progress { received, total });
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
            "The speech engine download looks incomplete ({received} bytes)."
        ));
    }

    let digest = format!("{:x}", hasher.finalize());
    if digest != ASSET_SHA256 {
        return Err(
            "The speech engine did not match its published checksum, so it was not installed."
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
        return Err("The speech engine archive has more files than expected.".to_string());
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
            return Err("The speech engine archive is larger than expected.".to_string());
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

/// How many recognition models are installed.
pub fn installed_models() -> usize {
    let mut found = 0;
    count_models(&crate::app_paths::models_root().join("stt"), &mut found);
    found
}

/// Removes the engine, refusing while its models can still use it.
///
/// A recognition model for which the runtime is gone fails at record time with
/// a message about an engine rather than about the delete that caused it, so
/// the delete is what refuses first -- the same rule the llama.cpp
/// engines' uninstall applies to models that name a missing build.
pub fn uninstall() -> Result<(), String> {
    if installed_models() > 0 {
        return Err(
            "Whisper models are installed. Delete them from the Speech to text page first."
                .to_string(),
        );
    }
    let directory = install_dir();
    if !directory.exists() {
        return Err("The speech engine is not installed.".to_string());
    }
    std::fs::remove_dir_all(&directory)
        .map_err(|error| format!("The speech engine could not be removed: {error}"))
}

fn count_models(directory: &Path, found: &mut usize) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            count_models(&path, found);
            continue;
        }
        if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("bin"))
        {
            *found += 1;
        }
    }
}
