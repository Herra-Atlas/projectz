//! A recognition model kept loaded.
//!
//! whisper.cpp's CLI reads the model, transcribes one file and exits, so the
//! weights are gone the moment the words are back. Its server does stay: one
//! process, one model, an HTTP door. This is that process.
//!
//! **Started on demand, never on install.** A resident model costs hundreds of
//! megabytes for as long as the app runs, so it exists only when the user asked
//! for it and only for the model they chose. A change of model stops it; the
//! next recording starts the new one.
//!
//! Parakeet is not served here: `whisper-server` reads whisper.cpp models only,
//! so a Parakeet file keeps using the CLI whatever the switch says.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::audio::engines::whisper as engine;

/// Where the resident engine listens. Beside the two audio.cpp engines and
/// llama-server, which are the other local servers.
const PORT: u16 = 43_129;
/// A cold model is read from disk before the port opens; a large one on a slow
/// drive takes a while.
const READY: Duration = Duration::from_secs(120);
const REQUEST: Duration = Duration::from_secs(300);

struct Resident {
    model: PathBuf,
    child: Child,
}

impl Drop for Resident {
    fn drop(&mut self) {
        // The model is memory: if this process outlives the one that started it,
        // nothing else will ever reclaim it.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

static SERVER: Mutex<Option<Resident>> = Mutex::new(None);

/// The model the resident engine is holding, if one is running.
pub fn running_model() -> Option<PathBuf> {
    SERVER
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(|resident| resident.model.clone()))
}

/// Stops the resident engine. Safe to call when none is running.
pub fn stop() {
    if let Ok(mut slot) = SERVER.lock() {
        slot.take();
    }
}

/// Transcribes one recording through the resident engine, starting it first if
/// the model on disk is not the one it is holding.
pub async fn transcribe(
    path: &Path,
    executable: &Path,
    samples: &[i16],
    language: Option<&str>,
) -> Result<String, String> {
    let wav = super::whisper::write_wav(samples)?;
    let bytes =
        std::fs::read(&wav).map_err(|error| format!("The recording could not be read: {error}"))?;
    let _ = std::fs::remove_file(&wav);

    let path = path.to_path_buf();
    let executable = executable.to_path_buf();
    tokio::task::spawn_blocking(move || start(&path, &executable))
        .await
        .map_err(|error| format!("The resident engine could not be started: {error}"))??;

    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|error| format!("The recording could not be sent: {error}"))?;
    let mut form = reqwest::multipart::Form::new()
        .part("file", part)
        // JSON, because the text is one field of it; `text` would answer a plain
        // body and lose the chance to tell a refusal from an empty transcript.
        .text("response_format", "json");
    if let Some(language) = language {
        form = form.text("language", language.to_string());
    }

    let response = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{PORT}/inference"))
        .multipart(form)
        .timeout(REQUEST)
        .send()
        .await
        .map_err(|error| format!("The resident engine could not be reached: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "The resident engine refused that recording ({}).",
            response.status()
        ));
    }
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|error| format!("The resident engine's answer could not be read: {error}"))?;
    let text = body["text"].as_str().unwrap_or_default();
    Ok(super::whisper::clean(text))
}

/// Starts the server for one model, or does nothing when it already holds it.
fn start(model: &Path, executable: &Path) -> Result<(), String> {
    if running_model().as_deref() == Some(model) {
        return Ok(());
    }
    stop();
    // Our own child was just killed, so give the port a moment to come back
    // before deciding that somebody else has it. A server left by an earlier app
    // run would otherwise make this one exit on bind, and the message would blame
    // the model for it.
    let freed = Instant::now() + Duration::from_secs(3);
    while std::net::TcpStream::connect(("127.0.0.1", PORT)).is_ok() {
        if Instant::now() > freed {
            return Err(format!(
                "Port {PORT} is still in use, most likely by a recognition server left running by an earlier app session. Close it and try again."
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    // The engine's own account of a failure is the only useful thing to report:
    // "the port did not open" describes my wait, not its reason. Written to disk
    // because a message has room for the last lines only.
    let log = std::env::temp_dir().join(format!(
        "projectz-whisper-server-{}.log",
        std::process::id()
    ));
    let stderr = std::fs::File::create(&log)
        .map_err(|error| format!("The engine's log could not be opened: {error}"))?;

    let mut command = Command::new(executable);
    command
        .current_dir(engine::install_dir())
        .arg("-m")
        .arg(model)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(PORT.to_string())
        .arg("-t")
        .arg(super::whisper::threads().to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr));
    crate::child::hide_console(&mut command);

    let mut child = command
        .spawn()
        .map_err(|error| format!("The resident engine could not be started: {error}"))?;
    let deadline = Instant::now() + READY;
    loop {
        if std::net::TcpStream::connect(("127.0.0.1", PORT)).is_ok() {
            break;
        }
        // An engine that died has nothing to wait for; saying so at once is the
        // difference between a wrong message in two minutes and a right one now.
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!(
                "The resident engine stopped at once ({}). {}",
                status,
                last_lines(&log)
            ));
        }
        if Instant::now() > deadline {
            let reason = last_lines(&log);
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "The resident engine did not open port {PORT} within {} seconds. {reason}",
                READY.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if let Ok(mut slot) = SERVER.lock() {
        *slot = Some(Resident {
            model: model.to_path_buf(),
            child,
        });
    }
    Ok(())
}

/// The last few lines the engine wrote, for a message that has one line.
fn last_lines(log: &Path) -> String {
    let Ok(text) = std::fs::read_to_string(log) else {
        return "Its log could not be read.".to_string();
    };
    let tail: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .rev()
        .take(3)
        .collect();
    if tail.is_empty() {
        "It wrote nothing before stopping.".to_string()
    } else {
        let ordered: Vec<&str> = tail.into_iter().rev().collect();
        format!("It said: {}", ordered.join(" / "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Starts the engine the way the app does, hidden console and all:
    ///
    /// ```text
    /// WHISPER_SERVER=…\whisper-server.exe WHISPER_MODEL=…\ggml-*.bin \
    ///   cargo test --lib the_resident_engine_opens_its_port -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "starts the recognition engine"]
    fn the_resident_engine_opens_its_port() {
        // The bug this guards: the engine was started from the *model's* CLI name,
        // which answers server arguments with its help text and exits 0.
        assert!(
            engine::executable(crate::audio::engines::whisper::SERVER_EXE).is_some(),
            "the installed engine has no whisper-server.exe"
        );
        let executable = std::env::var("WHISPER_SERVER").expect("WHISPER_SERVER");
        let model = std::env::var("WHISPER_MODEL").expect("WHISPER_MODEL");
        start(Path::new(&model), Path::new(&executable)).expect("the engine should start");
        stop();
    }
}
