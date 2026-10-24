//! Speech synthesis.
//!
//! One voice, one command surface: `tts_voice` says whether it is installed and
//! at what rate, `tts_speak` turns text into raw 16-bit mono PCM. Recognition
//! lives beside this in `stt/`, not inside it -- the two share a settings page
//! and nothing else.
//!
//! Two engines resolve here: Piper, the one this app shipped with, through
//! `piper.rs`'s per-sentence process; and audio.cpp, through `audiocpp.rs`'s HTTP
//! call to its server. `voices.rs` is what tells them apart, and nothing else in
//! this folder ever does.

pub mod audiocpp;
pub mod commands;
pub mod piper;
pub mod settings;
pub mod text;
pub mod voices;
