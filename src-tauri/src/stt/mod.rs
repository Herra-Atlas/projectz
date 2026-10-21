//! Speech recognition.
//!
//! The engine is a whisper.cpp build downloaded from this project's own release,
//! because a Vulkan build cannot be produced on a user's machine and is too large
//! to bundle. Models are ggml `.bin` files installed under `models/stt`.
//!
//! The pieces are separate on purpose: `engine` fetches and verifies the build,
//! `model` decides which installed model transcribes, `capture` owns the
//! microphone for exactly as long as one recording, and `transcribe` runs the
//! engine's CLI on the result. Nothing but `commands` knows the order.

pub mod capture;
pub mod commands;
pub mod engine;
pub mod model;
pub mod transcribe;
