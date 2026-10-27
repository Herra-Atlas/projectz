//! Where an engine's executables live, and the two engines this app knows.
//!
//! `whisper` is the build this app ships with and treats as a fact; `audiocpp`
//! is installed from a live GitHub release catalog, the same way the llama.cpp
//! engines are (`ai/local/engines.rs`). Both install under the same root the
//! llama.cpp engines use, so one uninstall pass (`system.rs`'s engines clearing)
//! covers the three of them.

pub mod audiocpp;
pub mod commands;
pub mod espeak;
pub mod openwakeword;
pub mod piper;
pub mod sherpa;
pub mod whisper;

/// The root every engine install sits under: `<data>/engines/<id>/`.
///
/// One directory per engine build, named after where it came from, so two
/// builds never overwrite each other and an uninstall is one `remove_dir_all`.
pub fn engines_root() -> std::path::PathBuf {
    crate::app_paths::data_root().join("engines")
}

/// Guards on an engine id, which becomes a path segment.
///
/// The same character set the llama.cpp engines' `installed_dir` accepts -- the
/// one check is written in both places rather than shared, because the two
/// sides disagree on nothing but which repo they serve, and sharing a constant
/// across features costs more than two six-line bodies.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_'))
}
