//! Talking to a running `audiocpp_server`.
//!
//! Three calls, in the shapes the server documents:
//!
//! * `GET /health` -- whether the process is up, which is also the only thing a
//!   status needs. No body is expected, only a status code.
//! * `POST /v1/audio/speech` -- the OpenAI speech shape, with `wav` requested so
//!   the answer carries its own sample rate in the header. The header is what
//!   `pcm_from_wav` reads, so no rate is ever guessed; it is then stripped, and
//!   the raw samples are what the frontend's player takes.
//! * `POST /v1/audio/transcriptions` -- the OpenAI transcription shape, the
//!   microphone's WAV uploaded as multipart, and the server answers `text`.
//!
//! The `model` both calls carry is the id the server config gave the model, not
//! its path: the config is what introduces a file on disk to the engine, and
//! after that the id is the address.

use std::path::Path;

use serde_json::{json, Value};

/// The speech body's normalized fields -- the ones the engine takes at the top
/// level rather than under `options`. The list is the endpoint's own contract
/// (its OpenAI-compatible shape): the style instruction, the voice pickers and
/// the cloning pair, plus the generation-shaping aliases.
const NORMALIZED_FIELDS: &[&str] = &[
    "instruct",
    "instructions",
    "voice",
    "voice_ref",
    "reference_audio",
    "reference_text",
    "seed",
    "max_tokens",
];

/// The address every call is made against.
///
/// Deliberately the loopback, and deliberately a port beside the local model's
/// (`43127`): an audio.cpp server is a third program on this machine, and two
/// servers bound to the same port would have failed noisily rather than quietly.
pub fn base_url() -> String {
    "http://127.0.0.1:43126".to_string()
}

/// The client for one request round. A new builder per call is what the engine
/// catalog already does, and one request per speech or transcription makes a
/// shared handle worth nothing here.
fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        // A client that cannot be built is a TLS-init failure, which is not a
        // per-request problem; the error is the honest answer either way.
        .unwrap_or_default()
}

/// The error the server meant, read out of a failed response.
///
/// The server answers a refused request with a JSON error body -- `{"error":
/// ...}` or `{"message": ...}` -- whose words are exactly what a runtime
/// negotiator matches against ("Could not load eSpeak-ng", "unknown … session
/// option"). A bare `error_for_status()` folds all of that into "500", which
/// is what made the first Kokoro request look like a mystery instead of a
/// missing phonemizer. Empty bodies fall back to a pointer at the logs.
async fn status_error(response: reqwest::Response) -> String {
    let body = response.text().await.unwrap_or_default();
    let parsed: Value = serde_json::from_str(&body).ok().unwrap_or(Value::Null);
    let message = parsed
        .get("error")
        .and_then(Value::as_str)
        .or_else(|| parsed.get("message").and_then(Value::as_str))
        .map(str::to_string)
        .unwrap_or_else(|| body.trim().to_string());
    if message.is_empty() {
        return "the server refused it for a reason it did not put in the response — the server's own logs (right panel) say what".to_string();
    }
    message.chars().take(300).collect()
}

/// Whether the server is answering.
pub async fn health() -> bool {
    let result = reqwest::get(format!("{}/health", base_url())).await;
    matches!(result, Ok(response) if response.status().is_success())
}

/// Synthesizes `text` with one installed model, and returns raw samples.
///
/// `speed` is the reader's pace, `1.0` the model's own, carried through the
/// OpenAI `speed` field. `None` sends nothing, so a model that has no opinion
/// about speed is never told one.
///
/// `request_options` is what the user set for this model -- its family spec's
/// names sent verbatim, plus anything else stored that is not a load option.
/// The engine validates its own omissions, so nothing here filters: what the
/// user stored reaches the server.
pub async fn speech(
    model_id: &str,
    text: &str,
    speed: Option<f32>,
    request_options: Option<serde_json::Value>,
) -> Result<(Vec<u8>, u32), String> {
    let mut body = json!({
        "model": model_id,
        "input": text,
    });
    if let Some(speed) = speed {
        body["speed"] = json!(speed);
    }
    // WAV is the endpoint's default answer; the header is what a rate is read
    // from, so asking for a format it does not offer is one more thing to fail.
    //
    // The user's request options split by the engine's contract: the fields it
    // normalizes to the body's top level (`instruct`, `voice` and friends) go
    // there -- proven against the running server, `voice_ref` under `options`
    // is refused as an unknown option while the same key at the top level
    // synthesizes -- and everything else rides the `options` overrides block
    // the contract gives them.
    if let Some(options) = request_options.filter(|options| options.is_object()) {
        let mut overrides = serde_json::Map::new();
        for (name, value) in options.as_object().expect("checked above") {
            if value.is_null() {
                continue;
            }
            if NORMALIZED_FIELDS.contains(&name.as_str()) {
                body[name.clone()] = value.clone();
            } else {
                overrides.insert(name.clone(), value.clone());
            }
        }
        if !overrides.is_empty() {
            body["options"] = Value::Object(overrides);
        }
    }

    let response = http()
        .post(format!("{}/v1/audio/speech", base_url()))
        .json(&body)
        .send()
        .await
        .map_err(|error| format!("The speech engine could not be reached: {error}"))?;
    let bytes = read_audio(response).await?;
    let (rate, pcm) = pcm_from_wav(&bytes)?;
    if pcm.is_empty() {
        return Err("The speech engine produced no audio.".to_string());
    }
    Ok((pcm, rate))
}

/// Uploads a WAV recording to the server and returns its transcript.
///
/// `request_options` rides the form as JSON fields, the way the speech call
/// carries them -- a hotword list or a language override reaches the engine by
/// its own name.
pub async fn transcribe(
    model_id: &str,
    wav_path: &Path,
    request_options: Option<serde_json::Value>,
) -> Result<String, String> {
    let file = tokio::fs::read(wav_path)
        .await
        .map_err(|error| format!("The recording could not be read back: {error}"))?;
    let part = reqwest::multipart::Part::bytes(file)
        .file_name("recording.wav")
        .mime_str("audio/wav")
        .map_err(|error| format!("The recording could not be prepared: {error}"))?;
    let mut form = reqwest::multipart::Form::new()
        .text("model", model_id.to_string())
        .part("file", part);
    if let Some(options) = request_options.filter(|options| options.is_object()) {
        if let Some(object) = options.as_object() {
            for (name, value) in object {
                form = form.text(name.clone(), value.to_string());
            }
        }
    }

    #[derive(serde::Deserialize)]
    struct Transcript {
        text: String,
    }

    let response = http()
        .post(format!("{}/v1/audio/transcriptions", base_url()))
        .multipart(form)
        .send()
        .await
        .map_err(|error| format!("The speech engine could not be reached: {error}"))?;
    if !response.status().is_success() {
        let reason = status_error(response).await;
        return Err(format!(
            "The speech engine refused that recording: {reason}"
        ));
    }
    let transcript = response
        .json::<Transcript>()
        .await
        .map_err(|error| format!("The transcript could not be read: {error}"))?;
    Ok(transcript.text)
}

/// The server's status bytes, when the answer is not a success story.
async fn read_audio(response: reqwest::Response) -> Result<Vec<u8>, String> {
    if !response.status().is_success() {
        let reason = status_error(response).await;
        return Err(format!("The speech engine could not speak that: {reason}"));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("The speech was interrupted: {error}"))?;
    Ok(bytes.to_vec())
}

/// The raw samples a WAV holds, and the rate its header states.
///
/// Only 16-bit PCM is handled, because that is the only shape this app's player
/// takes and the `wav` format the speech call asks. Every offset is bounds
/// checked rather than sliced blind, so a truncated or encoded file reports
/// *what* was wrong with the header instead of panicking on it.
pub fn pcm_from_wav(bytes: &[u8]) -> Result<(u32, Vec<u8>), String> {
    const HEADER: usize = 44;
    if bytes.len() < HEADER || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("The speech engine did not answer a WAV file.".to_string());
    }
    let rate = u16::from_le_bytes([bytes[24], bytes[25]]) as u32;
    let channels = u16::from_le_bytes([bytes[22], bytes[23]]) as usize;
    let bits = u16::from_le_bytes([bytes[34], bytes[35]]);
    if rate == 0 {
        return Err("The speech engine did not state its sample rate.".to_string());
    }
    if bits != 16 {
        return Err(format!(
            "The speech engine answered {bits}-bit samples, which the player cannot take."
        ));
    }
    // Mono is what the player asked for; stereo frames are interleaved, so the
    // right channel starts beside the left one and taking a frame's first two
    // bytes is what turns one into the other cheaply.
    let frame_bytes = ((bits / 8) as usize).max(2) * channels.max(1);
    let pcm = bytes[HEADER..]
        .chunks(frame_bytes)
        .flat_map(|frame| {
            let wide = frame.len().min(2);
            frame[..wide].iter().copied()
        })
        .collect::<Vec<u8>>();
    Ok((rate, pcm))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wav_is_read_back_as_samples_and_rate() {
        // 44-byte header + one mono 16-bit sample, silent.
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&40u32.to_le_bytes());
        wav.extend_from_slice(b"WAVE");
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&1u16.to_le_bytes()); // mono
        wav.extend_from_slice(&24_000u32.to_le_bytes());
        wav.extend_from_slice(&48_000u32.to_le_bytes()); // byte rate
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&2u32.to_le_bytes());
        wav.extend_from_slice(&42i16.to_le_bytes());
        wav.extend_from_slice(&(-1i16).to_le_bytes());

        let (rate, pcm) = pcm_from_wav(&wav).unwrap();
        assert_eq!(rate, 24_000);
        assert_eq!(pcm.len(), 4);
        assert_eq!(i16::from_le_bytes([pcm[0], pcm[1]]), 42);
        assert_eq!(i16::from_le_bytes([pcm[2], pcm[3]]), -1);
    }

    #[test]
    fn a_truncated_header_refuses_rather_than_slices() {
        assert!(pcm_from_wav(&b"RIFF".to_vec()).is_err());
        assert!(pcm_from_wav(&[0u8; 45]).is_err());
    }
}
