//! Speech synthesis.
//!
//! One voice, one command surface: `tts_voice` says whether it is installed and
//! at what rate, `tts_speak` turns text into raw 16-bit mono PCM. Recognition will
//! live beside this in `stt/`, not inside it -- the two share a settings page and
//! nothing else.

pub mod commands;
pub mod piper;
pub mod settings;
pub mod text;
pub mod voice;
