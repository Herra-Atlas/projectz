//! The openWakeWord engine: the runtime, and the two models every word shares.
//!
//! **Three files, pinned.** The collection on GitHub ships only a 205 KB
//! classifier head per word; the mel spectrogram that turns audio into frames
//! and the embedding CNN that turns frames into features are shared by every
//! head and live in openWakeWord's own release. So the engine installs those two
//! plus an ONNX Runtime DLL, and a word download adds one small file beside
//! them.
//!
//! **Why a second runtime at all.** audio.cpp can run its own `wake` families,
//! but nothing can load the collection's graphs: the speaker engine's CLI has no
//! mode for them. ONNX Runtime is the thing that executes an arbitrary ONNX
//! graph, so it is what runs these.
//!
//! Nothing here knows about microphones or thresholds; `audio/wake` owns that.

use std::fs;
use std::io::Read;
use std::path::PathBuf;

use futures_util::StreamExt;

use super::engines_root;

pub const ID: &str = "openwakeword";

const RUNTIME_VERSION: &str = "1.31.0";
const MODEL_VERSION: &str = "v0.5.1";

/// The runtime archive Microsoft publishes. The DLL is extracted from it and the
/// rest is dropped: headers and import libraries are for people building
/// against it, which this app does not.
///
/// **The version is not free to bump alone.** `ort` refuses a runtime older than
/// the API it was built against, and this one is what the crate requires; the
/// build fails to start the runtime rather than misbehaving quietly if they drift.
const RUNTIME_URL: &str = "https://github.com/microsoft/onnxruntime/releases/download/v1.31.0/onnxruntime-win-x64-1.31.0.zip";
const RUNTIME_SHA256: &str = "3958d8a44984160692c1b4ffd0de3cb48f8c39f9d34bb31e433db06e30b82195";
const RUNTIME_BYTES: u64 = 82_887_349;
/// Where the DLL sits inside that archive. Matched by suffix so a version bump
/// that renames the top directory still installs.
const RUNTIME_ENTRY: &str = "/lib/onnxruntime.dll";

const MEL_SHA256: &str = "ba2b0e0f8b7b875369a2c89cb13360ff53bac436f2895cced9f479fa65eb176f";
const MEL_BYTES: u64 = 1_087_958;
const EMBEDDING_SHA256: &str = "70d164290c1d095d1d4ee149bc5e00543250a7316b59f31d056cff7bd3075c1f";
const EMBEDDING_BYTES: u64 = 1_326_578;

/// The largest file this installer will hold in memory, in bytes. The archive is
/// 63 MB by design; anything far past it is not the archive.
const MAX_BYTES: u64 = 96 * 1024 * 1024;

pub fn install_dir() -> PathBuf {
    engines_root().join(ID)
}

pub fn runtime() -> PathBuf {
    install_dir().join("onnxruntime.dll")
}

pub fn mel() -> PathBuf {
    install_dir().join("melspectrogram.onnx")
}

pub fn embedding() -> PathBuf {
    install_dir().join("embedding_model.onnx")
}

/// Where a downloaded word's head goes. One file per word, beside the shared
/// models, named by the word.
pub fn heads_dir() -> PathBuf {
    install_dir().join("words")
}

pub fn is_installed() -> bool {
    runtime().is_file() && mel().is_file() && embedding().is_file()
}

pub fn installed_version() -> Option<String> {
    is_installed().then(|| format!("{RUNTIME_VERSION} · openWakeWord {MODEL_VERSION}"))
}

/// Installs the runtime and the two shared models, verifying each by size and
/// digest before anything is moved into place.
pub async fn install(_app: &tauri::AppHandle) -> Result<(), String> {
    let staging = install_dir().with_extension("staging");
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)
        .map_err(|error| format!("The install folder could not be made: {error}"))?;

    let archive = fetch(RUNTIME_URL, RUNTIME_BYTES, RUNTIME_SHA256).await?;
    let dll = runtime_from(&archive)?;
    fs::write(staging.join("onnxruntime.dll"), dll)
        .map_err(|error| format!("The runtime could not be written: {error}"))?;

    let mel = fetch(
        &format!("https://github.com/dscripka/openWakeWord/releases/download/{MODEL_VERSION}/melspectrogram.onnx"),
        MEL_BYTES,
        MEL_SHA256,
    )
    .await?;
    fs::write(staging.join("melspectrogram.onnx"), mel)
        .map_err(|error| format!("The mel model could not be written: {error}"))?;

    let embedding = fetch(
        &format!("https://github.com/dscripka/openWakeWord/releases/download/{MODEL_VERSION}/embedding_model.onnx"),
        EMBEDDING_BYTES,
        EMBEDDING_SHA256,
    )
    .await?;
    fs::write(staging.join("embedding_model.onnx"), embedding)
        .map_err(|error| format!("The embedding model could not be written: {error}"))?;

    let target = install_dir();
    let _ = fs::remove_dir_all(&target);
    fs::rename(&staging, &target)
        .map_err(|error| format!("The install could not be moved into place: {error}"))?;
    let _ = fs::create_dir_all(heads_dir());
    Ok(())
}

pub fn uninstall() -> Result<(), String> {
    let target = install_dir();
    if target.exists() {
        fs::remove_dir_all(&target)
            .map_err(|error| format!("The engine could not be removed: {error}"))?;
    }
    Ok(())
}

/// The runtime DLL out of Microsoft's archive, without unpacking the rest.
fn runtime_from(archive: &[u8]) -> Result<Vec<u8>, String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive))
        .map_err(|error| format!("The runtime archive could not be read: {error}"))?;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|error| format!("The runtime archive could not be read: {error}"))?;
        if !entry.name().ends_with(RUNTIME_ENTRY) {
            continue;
        }
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry
            .read_to_end(&mut bytes)
            .map_err(|error| format!("The runtime could not be extracted: {error}"))?;
        return Ok(bytes);
    }
    Err("The runtime archive did not contain onnxruntime.dll.".to_string())
}

/// One file, streamed, capped, and checked by size and digest before it is used.
async fn fetch(url: &str, expected_bytes: u64, expected_sha256: &str) -> Result<Vec<u8>, String> {
    let response = reqwest::get(url)
        .await
        .map_err(|error| format!("The download could not be started: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "The download was refused with {}.",
            response.status()
        ));
    }
    let mut stream = response.bytes_stream();
    let mut body: Vec<u8> = Vec::with_capacity(expected_bytes as usize);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("The download broke off: {error}"))?;
        body.extend_from_slice(&chunk);
        if body.len() as u64 > MAX_BYTES {
            return Err("The download was larger than expected and was stopped.".to_string());
        }
    }
    if body.len() as u64 != expected_bytes {
        return Err(format!(
            "The download was {} bytes, not the {expected_bytes} expected.",
            body.len()
        ));
    }
    let digest: String = {
        use sha2::{Digest, Sha256};
        Sha256::digest(&body)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    };
    if digest != expected_sha256 {
        return Err("The download did not match its published digest.".to_string());
    }
    Ok(body)
}
