//! Runs `piper.exe` once per request.
//!
//! # Why a process rather than a linked library
//!
//! Piper ships as a console program whose ONNX runtime is a DLL of its own, so
//! spawning it *is* the integration: no build step, no C++ toolchain, and a voice
//! is two files on disk. The cost is one process launch per message, measured at
//! roughly 10 ms here, which is less than the synthesis it asks for.
//!
//! # Why the text goes in on stdin
//!
//! A command line has a length limit and a message does not. Piper reads stdin as
//! a script of sentences and writes one continuous stream, which is exactly what
//! a player wants.
//!
//! # Why the writer is a thread
//!
//! `stdout` is a pipe with a small buffer. Writing a long message while the child
//! fills that pipe deadlocks both ends, so the text is written from its own
//! thread and this one only reads. Both pipes drain, so neither side waits.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A resolved, usable Piper voice: every path this process needs, checked.
///
/// Lives beside the process that consumes it, rather than in the shared
/// registry: every field here is Piper's business (which executable, where its
/// `espeak-ng-data` sits, which model and at what rate), and the registry
/// resolves *which* file, not *how* it runs.
pub struct Voice {
    pub program: PathBuf,
    /// The child process' working directory, which is also where its data sits.
    pub home: PathBuf,
    pub model: PathBuf,
    /// The rate of the PCM Piper returns, read from the config beside the model.
    pub sample_rate: u32,
}

/// The Piper voice a `.onnx` model resolves to, naming what is missing.
///
/// Checks before resolve rather than after: Piper is this app's default voice,
/// so these are the errors a user sees on an ordinary day, and each names the
/// file it wants rather than reporting a generic failure.
pub fn resolve_voice(model: &Path) -> Result<Voice, String> {
    let home = super::voices::install_dir();
    let program = home.join("piper.exe");
    if !program.is_file() {
        return Err(format!(
            "Piper is not installed. Expected {}.",
            program.display()
        ));
    }
    if !model.is_file() {
        return Err(format!(
            "The voice is missing. Expected {}.",
            model.display()
        ));
    }

    // `with_extension` replaces the last component, so `x.onnx` becomes
    // `x.onnx.json` -- the name Piper writes beside the model.
    let config = model.with_extension("onnx.json");
    if !config.is_file() {
        return Err(format!(
            "The voice's settings are missing. Expected {}.",
            config.display()
        ));
    }

    Ok(Voice {
        program,
        home,
        model: model.to_path_buf(),
        sample_rate: super::voices::piper_sample_rate(&config)?,
    })
}

/// A message long enough to stop being speech and start being an audio file.
///
/// The cap is on the *output*: at 16 kHz mono a minute of speech is under 2 MB,
/// so this is many minutes, and a message that could exceed it is refused rather
/// than allowed to grow until the process is killed.
const MAX_PCM_BYTES: usize = 32 * 1024 * 1024;

/// Turns `text` into raw 16-bit little-endian mono PCM at `voice.sample_rate`.
///
/// `speed` is the reader's pace: 1.0 is the voice's own. Piper takes it as
/// `length_scale`, which is the *duration* of a phoneme -- so it is the inverse,
/// and 2.0 as a speed is 0.5 as a length.
pub fn synthesize(voice: &Voice, text: &str, speed: f32) -> Result<Vec<u8>, String> {
    let length_scale = 1.0 / if speed > 0.0 { speed } else { 1.0 };

    let mut command = Command::new(&voice.program);
    command
        // Piper resolves its DLLs and `espeak-ng-data` relative to itself, and a
        // Windows child inherits the parent's directory rather than its own.
        .current_dir(&voice.home)
        // Named explicitly as well as found through the directory: a differently
        // packaged piper looks elsewhere, and one argument removes the question.
        .arg("--espeak_data")
        .arg(voice.home.join("espeak-ng-data"))
        .arg("--model")
        .arg(&voice.model)
        // Long form on purpose: this is the one argument whose meaning is not
        // obvious from the flag, and the next reader should not have to look it up.
        .arg("--length_scale")
        .arg(format!("{length_scale:.3}"))
        // Raw samples on stdout. A WAV header would only be parsed again to arrive
        // at the rate the config already states. Spelled with the underscore the
        // release's own help prints; `--output-raw` is rejected outright.
        .arg("--output_raw")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Piper is a console program: without this, every sentence flashes a black
    // window over the chat.
    crate::child::hide_console(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("Piper could not be started: {error}"))?;

    let script = text.to_string();
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Piper did not open its input.".to_string())?;

    let writer = std::thread::spawn(move || {
        // A failure here means piper stopped reading; the exit status below is the
        // error worth reporting, so this one is dropped on purpose.
        let _ = stdin.write_all(script.as_bytes());
        // Ends the script. Without it piper waits for more sentences.
        let _ = stdin.write_all(b"\n");
    });

    let mut pcm = Vec::new();
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Piper did not open its output.".to_string())?;
    stdout
        .by_ref()
        .take(MAX_PCM_BYTES as u64)
        .read_to_end(&mut pcm)
        .map_err(|error| format!("Could not read the speech Piper produced: {error}"))?;

    let _ = writer.join();

    // Read after stdout has drained: piper writes a handful of log lines, so the
    // pipe cannot fill while the larger stream above is being consumed.
    let mut stderr = String::new();
    if let Some(mut handle) = child.stderr.take() {
        let _ = handle.read_to_string(&mut stderr);
    }

    let status = child
        .wait()
        .map_err(|error| format!("Piper could not be waited on: {error}"))?;
    if !status.success() {
        let reason = stderr
            .lines()
            .last()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .unwrap_or("no reason given");
        return Err(format!("Piper could not speak that: {reason}"));
    }
    if pcm.is_empty() {
        return Err("Piper produced no audio.".to_string());
    }
    Ok(pcm)
}
