//! Everything that speaks and listens.
//!
//! One umbrella for the three engines' worth of sound, because the features it
//! holds share more than a settings page: a microphone, a WAV on disk, a voice
//! identified by its path, and a folder on disk that is the truth about what is
//! installed. The pieces:
//!
//! * `tts` -- reading a reply aloud. Piper is the engine it came with and stays
//!   the default; `audiocpp` serves the same ask for a second engine's voices.
//! * `stt` -- the composer's microphone. whisper.cpp is its engine and stays the
//!   default for the same reason, and `audiocpp ASR` families resolve beside it.
//! * `engines` -- where executables are installed and verified. The whisper.cpp
//!   build keeps its pinned constants; audio.cpp is installed from a live GitHub
//!   release catalog, the way the llama.cpp engines are.
//! * `audiocpp` -- the audio.cpp engine itself. One server process serves both
//!   tasks, so its lifecycle and HTTP client live here once, and `tts` and `stt`
//!   call into it rather than each keeping a copy.
//!
//! Each engine's process machine owns its own folder; the dispatch between two
//! engines for one voice or one model happens one level up, in `tts/voices` and
//! `stt/models`. A change of default engine is a change at the dispatch, then,
//! not a rewrite of the engines underneath it.

pub mod audiocpp;
pub mod engines;
pub mod stt;
pub mod tts;
pub mod voice;
pub mod wake;
