//! Turning recorded audio into words.
//!
//! # Why a process rather than a linked library
//!
//! The engine is installed on demand, so it cannot be linked at build time. A
//! process also keeps the model format private to whisper.cpp: the app hands it a
//! WAV and reads text back, and a new build of the engine needs no matching change
//! here.
//!
//! # Why a WAV on disk
//!
//! The engine's CLI takes a file, not a stream. The file is written to the system
//! temp directory under a unique name and removed immediately after, so two
//! recordings can never collide and nothing accumulates.

use std::io::Write;
use std::path::PathBuf;

use super::capture::SAMPLE_RATE;
use super::models::Model;
use crate::audio::engines::whisper as engine;

/// Transcribes one recording with the model the user chose.
///
/// `language` is a code the user set for this model, or `None` to let the engine
/// detect it. Detection is a second pass through the audio, so pinning it is both
/// faster and steadier for a microphone that only ever hears one language.
pub fn transcribe(
    model: &Model,
    samples: &[i16],
    language: Option<&str>,
) -> Result<String, String> {
    let executable = engine::executable(model.executable()).ok_or_else(|| {
        format!(
            "{} is missing from the speech engine. Reinstall it from the Local page.",
            model.executable()
        )
    })?;
    let wav = write_wav(samples)?;

    let mut command = std::process::Command::new(&executable);
    command
        // The engine resolves its DLLs beside itself, and a Windows child inherits
        // the parent's directory rather than its own.
        .current_dir(engine::install_dir())
        .arg("--model")
        .arg(&model.path)
        .arg("--file")
        .arg(&wav)
        .arg("--threads")
        .arg(threads().to_string())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if !model.parakeet {
        // Segment timestamps are for reading along, not for the composer.
        command.arg("--no-timestamps");
        if let Some(language) = language {
            command.arg("--language").arg(language);
        }
    }
    crate::child::hide_console(&mut command);

    let output = command
        .output()
        .map_err(|error| format!("The speech engine could not be started: {error}"))?;
    let _ = std::fs::remove_file(&wav);

    if !output.status.success() {
        let reason = String::from_utf8_lossy(&output.stderr);
        let last = reason
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("no reason given")
            .trim()
            .to_string();
        return Err(format!("The speech engine could not read that: {last}"));
    }

    Ok(clean(&String::from_utf8_lossy(&output.stdout)))
}

/// Half the cores, at least two: the engine scales poorly past that on a small
/// model and the rest of the app still has to run while it works.
pub(crate) fn threads() -> usize {
    std::thread::available_parallelism()
        .map(|count| (count.get() / 2).clamp(2, 8))
        .unwrap_or(2)
}

/// 16-bit mono PCM at the engine's rate, in a 44-byte RIFF header.
///
/// Public because the audio.cpp path hands the same file to its server over
/// HTTP: one recording, one WAV, whichever engine takes it.
pub fn write_wav(samples: &[i16]) -> Result<PathBuf, String> {
    let path = std::env::temp_dir().join(format!("projectz-{}.wav", uuid::Uuid::new_v4()));
    let data_len = (samples.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + samples.len() * 2);

    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    bytes.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }

    let mut file = std::fs::File::create(&path)
        .map_err(|error| format!("The recording could not be written: {error}"))?;
    file.write_all(&bytes)
        .map_err(|error| format!("The recording could not be written: {error}"))?;
    Ok(path)
}

/// The transcript, with the engine's bookkeeping removed.
///
/// Whisper answers silence with a bare `.` or a marker rather than nothing, and
/// either would arrive in the composer as a message that says nothing.
pub(crate) fn clean(raw: &str) -> String {
    let text = raw
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !line.starts_with("[BLANK_AUDIO]"))
        .collect::<Vec<_>>()
        .join(" ");
    if text.chars().all(|character| !character.is_alphanumeric()) {
        return String::new();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_silent_recording_has_nothing_to_say() {
        assert_eq!(clean(".\n"), "");
        assert_eq!(clean("[BLANK_AUDIO]\n"), "");
        assert_eq!(clean("   \n\n"), "");
    }

    #[test]
    fn the_transcript_keeps_its_sentences_on_one_line() {
        assert_eq!(
            clean(" Hello there.\n\n General Kenobi. "),
            "Hello there. General Kenobi."
        );
    }

    #[test]
    fn a_wav_header_states_the_rate_the_engine_expects() {
        let path = write_wav(&[0, 1000, -1000]).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..16], b"WAVEfmt ");
        assert_eq!(u16::from_le_bytes([bytes[22], bytes[23]]), 1);
        assert_eq!(
            u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]),
            SAMPLE_RATE
        );
        assert_eq!(bytes.len(), 44 + 6);
    }
}
