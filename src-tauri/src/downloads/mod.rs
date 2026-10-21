//! Downloading models and voices from HuggingFace.
//!
//! The page asks one question -- "what can I install for this kind" -- and the
//! installer answers one -- "put this file where the loader will look". Both are
//! per-kind rules held in `kinds.rs`, so adding recognition is one enum arm.

pub mod catalog;
pub mod client;
pub mod commands;
pub mod install;
pub mod jobs;
pub mod kinds;
