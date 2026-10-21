//! Speech recognition.
//!
//! The engine is a whisper.cpp build downloaded from this project's own release,
//! because a Vulkan build cannot be produced on a user's machine and is too large
//! to bundle. Models are ggml `.bin` files installed under `models/stt`.
//!
//! Transcription itself is not here yet: this half installs what it needs, so the
//! capture and the decode can be added without touching the installer.

pub mod commands;
pub mod engine;
