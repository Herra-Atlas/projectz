//! The sherpa-onnx engine: speaker embeddings, and only those.
//!
//! audio.cpp covers voices, transcription, wake words and voice activity, but
//! it has no speaker-embedding family, so "is this the person we enrolled"
//! cannot come from it. This engine is that one missing model: an ONNX
//! speaker-embedding net behind a C API, loaded at runtime only when voice
//! recognition is actually used.
//!
//! # Why pinned rather than catalogued
//!
//! audio.cpp publishes one build wave per release with digest-signed assets, so
//! its catalog can be live. sherpa-onnx's asset names change shape between
//! releases and the API this app calls is a C ABI -- a version silently
//! swapping the struct layout under us would be a crash, not an error. So the
//! release is pinned, the digest is written here, and the version is the one
//! this source was compiled against.
//!
//! # What is installed
//!
//! Three DLLs from the release archive plus one model. The archive ships two
//! dozen command-line tools; none of them are the API, and shipping them would
//! double the install for programs the app never runs.

use std::fs;
use std::path::PathBuf;

use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};

use super::engines_root;

/// The id, and the directory the engine installs into.
pub const ID: &str = "sherpa-onnx";

/// The release this source's FFI layer was written against.
const RELEASE: &str = "2.0.1";

/// The x64 shared build without TTS: the C API and its runtime, nothing else.
const ARCHIVE_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/v2.0.1/sherpa-onnx-v2.0.1-win-x64-shared-MD-MinSizeRel-no-tts.tar.bz2";
const ARCHIVE_SHA256: &str = "5520c214fae80ae40d37b3581bd67446d7c8ae8e3f25a6db916be1fc2860946a";
const ARCHIVE_BYTES: u64 = 18_047_922;

/// 3D-Speaker's ERes2Net base: 16 kHz, 192-dimensional embeddings, trained for
/// text-independent verification. English speech is well inside its domain even
/// though the training corpus is Chinese -- and it is the model sherpa's own
/// diarization examples pair with this build.
const MODEL_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx";
const MODEL_SHA256: &str = "1a331345f04805badbb495c775a6ddffcdd1a732567d5ec8b3d5749e3c7a5e4b";
const MODEL_BYTES: u64 = 39_593_761;

/// The model's file name once installed.
const MODEL_FILE: &str = "speaker-embedding.onnx";

/// The DLLs the C API needs. `sherpa-onnx-c-api.dll` calls into the C++ API and
/// ONNX Runtime, so both are load-bearing even though this app names neither.
const LIBRARIES: [&str; 3] = [
    "sherpa-onnx-c-api.dll",
    "sherpa-onnx-cxx-api.dll",
    "onnxruntime.dll",
];

pub fn install_dir() -> PathBuf {
    engines_root().join(ID)
}

fn staging_dir() -> PathBuf {
    engines_root().join("sherpa-onnx-staging")
}

/// The library the FFI layer loads.
pub fn library() -> PathBuf {
    install_dir().join(LIBRARIES[0])
}

/// The embedding model the engine runs.
pub fn model() -> PathBuf {
    install_dir().join(MODEL_FILE)
}

/// Installed means both halves are on disk: a library without a model cannot
/// answer, and a model without a library cannot run.
pub fn is_installed() -> bool {
    library().is_file() && model().is_file()
}

pub fn installed_version() -> Option<String> {
    is_installed().then(|| RELEASE.to_string())
}

/// Downloads, verifies, unpacks.
///
/// Everything is written into a staging directory and moved into place only
/// after both files verify, so a download that dies halfway leaves no install
/// that looks complete.
pub async fn install(app: &AppHandle) -> Result<(), String> {
    let staging = staging_dir();
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)
        .map_err(|error| format!("The staging folder could not be made: {error}"))?;

    // One bar across both files: the model is most of the download, and a bar
    // that restarted at zero halfway would read as a failure.
    let whole = ARCHIVE_BYTES + MODEL_BYTES;
    let archive = fetch(
        ARCHIVE_URL,
        ARCHIVE_BYTES,
        ARCHIVE_SHA256,
        "the sherpa-onnx build",
        app,
        0,
        whole,
    )
    .await?;
    unpack(&archive, &staging)?;
    if !LIBRARIES.iter().all(|name| staging.join(name).is_file()) {
        let _ = fs::remove_dir_all(&staging);
        return Err("The archive did not contain the sherpa-onnx C API libraries.".to_string());
    }

    let model = fetch(
        MODEL_URL,
        MODEL_BYTES,
        MODEL_SHA256,
        "the speaker model",
        app,
        ARCHIVE_BYTES,
        whole,
    )
    .await?;
    fs::write(staging.join(MODEL_FILE), model)
        .map_err(|error| format!("The speaker model could not be written: {error}"))?;

    let target = install_dir();
    let _ = fs::remove_dir_all(&target);
    fs::rename(&staging, &target)
        .map_err(|error| format!("The engine could not be moved into place: {error}"))
}

pub fn uninstall() -> Result<(), String> {
    let directory = install_dir();
    if !directory.exists() {
        return Err("The voice recognition engine is not installed.".to_string());
    }
    fs::remove_dir_all(&directory)
        .map_err(|error| format!("Could not remove the voice recognition engine: {error}"))
}

/// Reads the archive and writes just the libraries, flat into `target`.
///
/// Names are matched rather than paths trusted: the archive nests everything
/// under `bin/`, and a `..` in a member name would otherwise be an extraction
/// outside the staging folder.
fn unpack(archive: &[u8], target: &std::path::Path) -> Result<(), String> {
    let decoder = bzip2::read::BzDecoder::new(std::io::Cursor::new(archive));
    // Bound rather than chained: the entry iterator borrows the archive, and a
    // temporary would be dropped before the first member was read.
    let mut bundle = tar::Archive::new(decoder);
    let mut entries = bundle
        .entries()
        .map_err(|error| format!("The sherpa-onnx archive could not be read: {error}"))?;
    let mut written = 0usize;
    while let Some(entry) = entries.next() {
        let Ok(mut entry) = entry else { continue };
        let path = entry
            .path()
            .map_err(|error| format!("A member of the archive is unreadable: {error}"))?
            .to_path_buf();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !LIBRARIES.contains(&name) {
            continue;
        }
        // Beside each other, because the loader resolves the runtime from the
        // directory of the library it was given.
        entry
            .unpack(target.join(name))
            .map_err(|error| format!("{name} could not be written: {error}"))?;
        written += 1;
    }
    if written == 0 {
        return Err("The sherpa-onnx archive was empty.".to_string());
    }
    Ok(())
}

/// One verified download: size first (cheap, and catches a wrong asset), then
/// the digest.
async fn fetch(
    url: &str,
    expected_bytes: u64,
    expected_sha256: &str,
    what: &str,
    app: &AppHandle,
    offset: u64,
    whole: u64,
) -> Result<Vec<u8>, String> {
    // A ceiling as well as an equality check, so a redirect to something huge
    // cannot fill memory before the digest is compared.
    const CEILING: u64 = 128 * 1024 * 1024;
    if expected_bytes > CEILING {
        return Err(format!("{what} is larger than this app will download."));
    }
    let response = reqwest::get(url)
        .await
        .map_err(|error| format!("{what} could not be downloaded: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "{what} could not be downloaded: the server answered {}.",
            response.status()
        ));
    }
    let mut body: Vec<u8> = Vec::with_capacity(expected_bytes as usize);
    let mut stream = response.bytes_stream();
    // Reported every 64 KB rather than per chunk: a 40 MB model arrives in
    // thousands of chunks, and an event per chunk is a repaint per chunk.
    let mut reported = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("{what} stopped arriving: {error}"))?;
        body.extend_from_slice(&chunk);
        if body.len() as u64 > CEILING {
            return Err(format!("{what} is larger than expected."));
        }
        let received = body.len() as u64;
        if received - reported >= 64 * 1024 || received == expected_bytes {
            reported = received;
            let _ = app.emit(
                "engine-download-progress",
                serde_json::json!({
                    "id": ID,
                    "downloaded": offset + received,
                    "total": whole,
                }),
            );
        }
    }
    if body.len() as u64 != expected_bytes {
        return Err(format!(
            "{what} was {} bytes, expected {expected_bytes}.",
            body.len()
        ));
    }
    let digest: String = Sha256::digest(&body)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if digest != expected_sha256 {
        return Err(format!("{what} did not match its published digest."));
    }
    Ok(body)
}
