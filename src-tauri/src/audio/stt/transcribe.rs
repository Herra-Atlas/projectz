//! One recognition path for the microphone and for Telegram voice messages.
//!
//! Both engines get the same 16 kHz mono samples, so the only difference
//! between a recording and a downloaded Ogg Opus voice note is how the samples
//! are obtained. Keeping the dispatch here means a model selected on the Local
//! page is exactly the model Telegram uses.

use std::io::Cursor;

use opus_pure::{OggOpusReader, Trim, MAX_PACKET_SAMPLES};
use tauri::AppHandle;

use crate::database::Database;

use super::audiocpp;
use super::capture::MIN_SAMPLES;
use super::models::{self, Selected};
use super::whisper;

/// Transcribes microphone samples with the model selected on the Local page.
///
/// An empty return is an error here: every caller is waiting for words, and a
/// silent successful result would look like the message vanished.
pub async fn samples(
    app: &AppHandle,
    database: &Database,
    samples: Vec<i16>,
) -> Result<String, String> {
    if samples.len() < MIN_SAMPLES {
        return Err("Nothing was heard. Hold the microphone a little longer.".to_string());
    }
    let settings = models::settings(database);
    let selected = models::resolve(&settings)?;
    let language = settings.language_for(&selected.path()).map(str::to_string);

    let text = match selected {
        Selected::WhisperFile(model) => {
            // A resident engine holds the weights between recordings; without it
            // each press pays for reading the model from disk again.
            if settings.keep_loaded && !model.parakeet {
                // The server, not the model's own CLI: `whisper-cli` answers these
                // arguments with its help text and a successful exit.
                let executable =
                    crate::audio::engines::whisper::executable(
                        crate::audio::engines::whisper::SERVER_EXE,
                    )
                    .ok_or_else(|| {
                        "whisper-server is missing from the speech engine. Reinstall it from the Local page."
                            .to_string()
                    })?;
                super::resident::transcribe(
                    std::path::Path::new(&model.path),
                    &executable,
                    &samples,
                    language.as_deref(),
                )
                .await?
            } else {
                tokio::task::spawn_blocking(move || {
                    whisper::transcribe(&model, &samples, language.as_deref())
                })
                .await
                .map_err(|error| format!("Transcription was interrupted: {error}"))??
            }
        }
        Selected::Audiocpp(model) => {
            let wav = whisper::write_wav(&samples)?;
            let outcome = audiocpp::transcribe(&model, &wav, app, database).await;
            // Either way the temp file has done its job and must not survive.
            let _ = std::fs::remove_file(&wav);
            outcome?
        }
    };

    if text.is_empty() {
        return Err("Nothing was heard.".to_string());
    }
    Ok(text)
}

/// Decodes a Telegram voice note, then runs the same selected engine.
///
/// Telegram voice notes are Ogg Opus. Decoding in-process avoids requiring
/// ffmpeg and keeps the message's bytes out of a shell command line.
pub async fn voice(app: &AppHandle, database: &Database, bytes: Vec<u8>) -> Result<String, String> {
    let decoded = tokio::task::spawn_blocking(move || decode_ogg_opus(&bytes))
        .await
        .map_err(|error| format!("The voice message could not be decoded: {error}"))??;
    samples(app, database, decoded).await
}

fn decode_ogg_opus(bytes: &[u8]) -> Result<Vec<i16>, String> {
    let mut reader = OggOpusReader::new(Cursor::new(bytes))
        .map_err(|error| format!("This is not a valid Ogg Opus voice message: {error}"))?;
    let head = reader.head().clone();
    let channels = head.channel_count as usize;
    if channels == 0 {
        return Err("The voice message has no audio channels.".to_string());
    }
    let mut decoder = head
        .decoder(16_000)
        .map_err(|error| format!("The voice message could not be decoded: {error}"))?;
    let mut trim = Trim::new(&head, 16_000, channels)
        .map_err(|error| format!("The voice message could not be decoded: {error}"))?;
    let mut block = vec![0.0f32; MAX_PACKET_SAMPLES * channels];
    let mut mono = Vec::new();

    for packet in reader.packets() {
        let packet = packet.map_err(|error| format!("The voice message is incomplete: {error}"))?;
        let samples = decoder
            .decode(&packet.data, MAX_PACKET_SAMPLES, &mut block)
            .map_err(|error| format!("The voice message could not be decoded: {error}"))?;
        for frame in trim
            .keep(&packet, &block[..samples * channels])
            .chunks(channels)
        {
            let mixed = frame.iter().sum::<f32>() / channels as f32;
            let clamped = mixed.clamp(-1.0, 1.0);
            mono.push((clamped * i16::MAX as f32).round() as i16);
        }
    }

    if mono.len() < MIN_SAMPLES {
        return Err("The voice message is too short to transcribe.".to_string());
    }
    Ok(mono)
}

#[cfg(test)]
mod tests {
    use opus_pure::{Application, OggOpusWriter, OpusEncoder, OpusHead, MAX_PACKET_BYTES};

    use super::decode_ogg_opus;

    #[test]
    fn a_non_ogg_file_is_rejected() {
        assert!(decode_ogg_opus(b"not an ogg file").is_err());
    }

    #[test]
    fn a_real_ogg_opus_voice_note_decodes_to_samples() {
        let (rate, channels, frame) = (48_000, 1, 960);
        let pcm = vec![0.0f32; frame * 30];
        let mut encoder = OpusEncoder::new(rate, channels, Application::Audio).unwrap();
        let head = OpusHead::for_encoder(&encoder, rate as u32);
        let mut writer = OggOpusWriter::new(Vec::new(), head).unwrap();
        let mut packet = vec![0u8; MAX_PACKET_BYTES];
        for block in pcm.chunks_exact(frame * channels) {
            let written = encoder.encode(block, frame, &mut packet).unwrap();
            writer.write_packet(&packet[..written]).unwrap();
        }
        let file = writer.finish().unwrap();
        let samples = decode_ogg_opus(&file).unwrap();
        // Opus' pre-skip and end trim take a few milliseconds off the nominal
        // 600 ms. The decoder succeeding already proves the framing; this checks
        // that a real duration came back rather than a single packet.
        assert!(samples.len() >= 16_000 / 2);
    }
}
