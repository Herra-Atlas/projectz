//! The two HTTP calls the speech server answers, and nothing else.
//!
//! Recognition goes through the ordinary chat endpoint with one `input_audio`
//! part, because that is the shape `llama-server`'s multimodal parser already
//! understands and the model's own template already renders. Synthesis goes
//! through `/tts`, which was added for exactly this and returns a WAV body.
//!
//! Both deliberately re-read no state: the caller has already ensured the right
//! model is running, and a request that named a model would be a second place
//! "which model" is decided.

use std::time::Duration;

use base64::Engine as _;
use serde_json::json;

/// A 2B model generating speech from a long paragraph is slow, and a recording
/// longer than two minutes is refused before it gets here.
const REQUEST: Duration = Duration::from_secs(600);

fn base(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(REQUEST)
        .build()
        .map_err(|error| error.to_string())
}

/// Whether the server is up. A two-second budget: this runs on a poll.
pub async fn health_at(port: u16) -> bool {
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    else {
        return false;
    };
    client
        .get(format!("{}/health", base(port)))
        .send()
        .await
        .is_ok_and(|response| response.status().is_success())
}

/// Whether the server on `port` is already serving this exact model.
///
/// Asked of the server rather than of our own bookkeeping, because a chat model
/// can be the speech model too: LFM3 Audio answers `/tts` and takes
/// `input_audio`, so reusing that process is correct and saves a second copy of
/// the weights.
pub async fn serves(port: u16, path: &str) -> bool {
    let Some(served) = props_model(port).await else {
        return false;
    };
    let left = crate::ai::local::memory::normalise_windows_path(&served);
    let right = crate::ai::local::memory::normalise_windows_path(path);
    left == right
}

/// The model a running server has loaded, from its own `/props`.
pub async fn props_model(port: u16) -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .ok()?;
    let value: serde_json::Value = client
        .get(format!("{}/props", base(port)))
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    value["model_path"].as_str().map(str::to_string)
}

/// One recording in, its transcript out.
pub async fn transcribe(port: u16, wav: &[u8], language: Option<&str>) -> Result<String, String> {
    let encoded = base64::engine::general_purpose::STANDARD.encode(wav);
    // The model is a chat model that has to be told which job this is; it does
    // not infer "transcribe" from an attachment. The sentence is written here so
    // no user ever has to know it exists -- the language comes from the model's
    // own setting, and nothing is typed by hand.
    let instruction = match language.and_then(language_name) {
        Some(name) => format!("Perform ASR in {name}."),
        None => "Perform ASR in English.".to_string(),
    };
    let body = json!({
        "model": super::ALIAS,
        "messages": [
            { "role": "system", "content": instruction },
            {
                "role": "user",
                "content": [{
                    "type": "input_audio",
                    "input_audio": { "data": encoded, "format": "wav" }
                }]
            }
        ],
        // Greedy, as the model card asks for text output: a transcript is not a
        // place to be creative.
        "temperature": 0,
        "stream": false
    });

    let response = client()?
        .post(format!("{}/v1/chat/completions", base(port)))
        .json(&body)
        .send()
        .await
        .map_err(|error| format!("The speech model could not be reached: {error}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "The speech model refused that recording ({status}): {}",
            first_line(&body)
        ));
    }
    let value: serde_json::Value = response
        .json()
        .await
        .map_err(|error| format!("The transcript could not be read: {error}"))?;
    Ok(value["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .to_string())
}

/// Pins the speaker for every synthesis.
///
/// `/tts` has no voice setting -- llama.cpp logs "no speaker reference provided,
/// the model may behave randomly" and samples a speaker per request, so a reply
/// split into chunks changed voice from sentence to sentence. The seed is the one
/// lever it exposes that makes that choice repeatable: measured against the
/// running server, two calls without a seed differ, two calls with this one are
/// byte-identical.
const SPEAKER_SEED: u32 = 42;

/// Text in, a sample rate and raw mono samples out.
pub async fn speak(port: u16, text: &str) -> Result<(u32, Vec<i16>), String> {
    let response = client()?
        .post(format!("{}/tts", base(port)))
        .json(&json!({ "input": text, "seed": SPEAKER_SEED }))
        .send()
        .await
        .map_err(|error| format!("The speech model could not be reached: {error}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "The speech model refused that text ({status}): {}",
            first_line(&body)
        ));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("The spoken audio could not be read: {error}"))?;
    pcm_from_wav(&bytes)
}

/// The name the model was trained to hear in an instruction, from a stored code.
///
/// Region tags are dropped: `en-GB` and `en` are one language to this model, and
/// the voice/instruction sentence wants a name rather than a code.
fn language_name(code: &str) -> Option<&'static str> {
    let base = code.split(['-', '_']).next().unwrap_or(code);
    Some(match base.to_ascii_lowercase().as_str() {
        "en" => "English",
        "ar" => "Arabic",
        "da" => "Danish",
        "de" => "German",
        "el" => "Greek",
        "fi" => "Finnish",
        "fr" => "French",
        "he" => "Hebrew",
        "it" => "Italian",
        "ja" => "Japanese",
        "ko" => "Korean",
        "nl" => "Dutch",
        "pl" => "Polish",
        "pt" => "Portuguese",
        "es" => "Spanish",
        "sv" => "Swedish",
        "tr" => "Turkish",
        "zh" => "Chinese",
        _ => return None,
    })
}

/// A server's error body is a paragraph; the first line is the sentence.
fn first_line(body: &str) -> String {
    body.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("no reason given")
        .chars()
        .take(300)
        .collect()
}

/// Reads a WAV returned by the server into mono 16-bit samples.
///
/// The body is a container, and the player takes a buffer. Both sample shapes a
/// speech model may emit are accepted -- 16-bit PCM and 32-bit float, which is
/// what some TTS heads produce -- because guessing one and resampling the other
/// by hand would sound like a broken voice rather than an error.
fn pcm_from_wav(bytes: &[u8]) -> Result<(u32, Vec<i16>), String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("The speech model did not return a WAV file.".to_string());
    }

    let mut at = 12;
    let mut rate = 0u32;
    let mut channels = 1usize;
    let mut bits = 16usize;
    let mut float = false;
    let mut data: Option<&[u8]> = None;

    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]])
            as usize;
        let start = at + 8;
        // A truncated final chunk is read short rather than refused: the samples
        // that did arrive are still worth playing.
        let end = start.saturating_add(size).min(bytes.len());
        let body = &bytes[start..end];

        match id {
            b"fmt " => {
                if body.len() >= 16 {
                    let format = u16::from_le_bytes([body[0], body[1]]);
                    channels = u16::from_le_bytes([body[2], body[3]]).max(1) as usize;
                    rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
                    bits = u16::from_le_bytes([body[14], body[15]]) as usize;
                    // 3 is IEEE float; 0xFFFE is "extensible", whose sub-format
                    // is read from the first two bytes of the extension.
                    float = format == 3
                        || (format == 0xFFFE
                            && body.len() >= 18
                            && u16::from_le_bytes([body[8], body[9]]) == 3);
                }
            }
            b"data" => data = Some(body),
            _ => {}
        }
        // Chunks are word-aligned, so an odd payload carries one pad byte.
        at = start + size + (size & 1);
    }

    if rate == 0 {
        return Err("The spoken audio does not state its sample rate.".to_string());
    }
    let data = data.ok_or_else(|| "The spoken audio has no samples.".to_string())?;

    let samples: Vec<i16> = if float && bits == 32 {
        data.chunks_exact(4)
            .map(|chunk| {
                let value = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                (value.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
            })
            .collect()
    } else if bits == 16 {
        data.chunks_exact(2)
            .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
            .collect()
    } else {
        return Err(format!(
            "The spoken audio is {bits}-bit, which this player cannot read."
        ));
    };

    // The player builds a mono buffer, so a stereo body is folded rather than
    // played as alternating channels -- which is a voice at double speed.
    let pcm = if channels > 1 {
        samples
            .chunks(channels)
            .map(|frame| {
                let sum: i32 = frame.iter().map(|sample| *sample as i32).sum();
                (sum / frame.len() as i32) as i16
            })
            .collect()
    } else {
        samples
    };

    Ok((rate, pcm))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(bits: u16, float: bool, channels: u16, rate: u32, samples: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + samples.len() as u32).to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&(if float { 3u16 } else { 1 }).to_le_bytes());
        bytes.extend_from_slice(&channels.to_le_bytes());
        bytes.extend_from_slice(&rate.to_le_bytes());
        bytes.extend_from_slice(&(rate * channels as u32 * (bits as u32 / 8)).to_le_bytes());
        bytes.extend_from_slice(&(channels * (bits / 8)).to_le_bytes());
        bytes.extend_from_slice(&bits.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(samples.len() as u32).to_le_bytes());
        bytes.extend_from_slice(samples);
        bytes
    }

    /// A 24 kHz mono body is read as written.
    #[test]
    fn sixteen_bit_audio_keeps_its_rate_and_samples() {
        let body = wav(16, false, 1, 24_000, &[0x00, 0x40, 0x00, 0xc0]);
        let (rate, pcm) = pcm_from_wav(&body).expect("wav was read");
        assert_eq!(rate, 24_000);
        assert_eq!(pcm, vec![16_384, -16_384]);
    }

    /// Float bodies are converted rather than refused.
    #[test]
    fn float_audio_is_converted_to_samples() {
        let mut samples = Vec::new();
        samples.extend_from_slice(&0.5f32.to_le_bytes());
        samples.extend_from_slice(&(-0.5f32).to_le_bytes());
        let body = wav(32, true, 1, 24_000, &samples);
        let (_, pcm) = pcm_from_wav(&body).expect("float wav was read");
        assert!(pcm[0] > 16_000 && pcm[1] < -16_000);
    }

    /// Stereo is folded, not interleaved: playing both channels through a mono
    /// buffer sounds like double speed and an octave up.
    #[test]
    fn stereo_is_folded_to_one_channel() {
        let samples = [0x00, 0x10, 0x00, 0x30, 0x00, 0x20, 0x00, 0x40];
        let body = wav(16, false, 2, 24_000, &samples);
        let (_, pcm) = pcm_from_wav(&body).expect("stereo wav was read");
        assert_eq!(pcm, vec![8_192, 12_288]);
    }

    #[test]
    fn a_body_that_is_not_a_wav_is_refused_by_name() {
        assert!(pcm_from_wav(b"not a wav at all").is_err());
    }

    #[test]
    fn a_language_code_becomes_the_name_the_model_was_trained_on() {
        assert_eq!(language_name("en-GB"), Some("English"));
        assert_eq!(language_name("de"), Some("German"));
        assert_eq!(language_name("xx"), None);
    }
}
