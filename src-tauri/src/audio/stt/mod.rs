//! Speech recognition.
//!
//! whisper.cpp is the engine this app came with and stays the default: its
//! executable is a pinned build from this project's own release, because a
//! Vulkan build cannot be produced on a user's machine and is too large to
//! bundle. Its models are ggml `.bin` files installed under `models/stt`.
//!
//! audio.cpp's ASR families resolve beside it through `audiocpp.rs`: their
//! models are gguf packages under `models/audio`, and their way in is a call to
//! a running `audiocpp_server` rather than a process per recording.
//!
//! The pieces are separate on purpose: `engines::whisper` fetches and verifies
//! the build, `models` decides which installed model transcribes, `capture` owns
//! the microphone for exactly as long as one recording, and `whisper.rs` runs
//! that engine's CLI on the result. The dispatch between the two engines happens
//! in `transcribe`, which is shared by the microphone commands and Telegram
//! voice messages.

pub mod audiocpp;
pub mod capture;
pub mod commands;
pub mod models;
pub mod transcribe;
pub mod whisper;
