//! The sherpa-onnx speaker-embedding API, loaded at runtime.
//!
//! # Why FFI rather than a dependency
//!
//! The engine is an optional install that most users will never add, and it is
//! a native library with a C ABI. Linking it would make every build require it;
//! shelling out to its command-line tools would mean either a process per
//! verification (hundreds of milliseconds) or no verification at all -- the
//! only speech tool it ships that embeds audio is the diarizer, which spends
//! 12 seconds on a minute of audio. Calling the four functions below directly
//! gives one embedding in about ten milliseconds with no process to manage.
//!
//! # Why the layout is spelled out here
//!
//! These are the only two structures the API takes by value, and their field
//! order is part of the ABI. They are copied from `c-api.h` of the pinned
//! release (see `engines::sherpa`); the engine version is pinned for exactly
//! this reason, because a changed layout would be a crash rather than an error.

use std::ffi::{c_char, c_void, CString};
use std::path::Path;

use libloading::Library;

/// `SherpaOnnxSpeakerEmbeddingExtractorConfig`.
#[repr(C)]
struct Config {
    model: *const c_char,
    num_threads: i32,
    debug: i32,
    provider: *const c_char,
}

type Create = unsafe extern "C" fn(*const Config) -> *const c_void;
type Destroy = unsafe extern "C" fn(*const c_void);
type Dim = unsafe extern "C" fn(*const c_void) -> i32;
type CreateStream = unsafe extern "C" fn(*const c_void) -> *const c_void;
type IsReady = unsafe extern "C" fn(*const c_void, *const c_void) -> i32;
type Compute = unsafe extern "C" fn(*const c_void, *const c_void) -> *const f32;
type DestroyEmbedding = unsafe extern "C" fn(*const f32);
type AcceptWaveform = unsafe extern "C" fn(*const c_void, i32, *const f32, i32);
type InputFinished = unsafe extern "C" fn(*const c_void);
type DestroyStream = unsafe extern "C" fn(*const c_void);

/// The rate every speaker model in sherpa's catalog is trained on.
pub const SAMPLE_RATE: i32 = 16_000;

/// One loaded extractor.
pub struct Speaker {
    // Declared first and never read: dropping it drops the library, and the
    // function pointers below are only valid while it lives.
    _library: Library,
    create_stream: CreateStream,
    is_ready: IsReady,
    compute: Compute,
    dim: Dim,
    destroy_embedding: DestroyEmbedding,
    accept: AcceptWaveform,
    input_finished: InputFinished,
    destroy_stream: DestroyStream,
    destroy: Destroy,
    handle: *const c_void,
}

// The handle is owned by this struct, and every method takes `&self` with the
// caller serialising access behind a mutex: the C API has no thread guarantees,
// and the pointer is never used without that guard.
unsafe impl Send for Speaker {}

impl Speaker {
    /// Loads the library from `directory` and creates the extractor.
    ///
    /// The library is loaded with an altered search path so ONNX Runtime is
    /// found beside it rather than in the app's own folder.
    pub fn open(directory: &Path, model: &Path) -> Result<Self, String> {
        let path = directory.join("sherpa-onnx-c-api.dll");
        if !path.is_file() {
            return Err("The voice recognition engine is not installed.".to_string());
        }
        let library = load_library(&path)?;
        unsafe {
            let create: Create = symbol(&library, "SherpaOnnxCreateSpeakerEmbeddingExtractor")?;
            let model = CString::new(model.to_string_lossy().as_bytes())
                .map_err(|_| "The speaker model path is not usable.".to_string())?;
            let provider = CString::new("cpu").expect("a literal without nul bytes");
            let config = Config {
                model: model.as_ptr(),
                num_threads: 1,
                debug: 0,
                provider: provider.as_ptr(),
            };
            let handle = create(&config);
            if handle.is_null() {
                return Err("The speaker model could not be loaded.".to_string());
            }
            Ok(Self {
                create_stream: symbol(&library, "SherpaOnnxSpeakerEmbeddingExtractorCreateStream")?,
                is_ready: symbol(&library, "SherpaOnnxSpeakerEmbeddingExtractorIsReady")?,
                compute: symbol(
                    &library,
                    "SherpaOnnxSpeakerEmbeddingExtractorComputeEmbedding",
                )?,
                dim: symbol(&library, "SherpaOnnxSpeakerEmbeddingExtractorDim")?,
                destroy_embedding: symbol(
                    &library,
                    "SherpaOnnxSpeakerEmbeddingExtractorDestroyEmbedding",
                )?,
                accept: symbol(&library, "SherpaOnnxOnlineStreamAcceptWaveform")?,
                input_finished: symbol(&library, "SherpaOnnxOnlineStreamInputFinished")?,
                destroy_stream: symbol(&library, "SherpaOnnxDestroyOnlineStream")?,
                destroy: symbol(&library, "SherpaOnnxDestroySpeakerEmbeddingExtractor")?,
                _library: library,
                handle,
            })
        }
    }

    /// One embedding for one clip of 16 kHz mono audio.
    pub fn embed(&self, samples: &[f32]) -> Result<Vec<f32>, String> {
        if samples.is_empty() {
            return Err("That clip was empty.".to_string());
        }
        unsafe {
            let stream = (self.create_stream)(self.handle);
            if stream.is_null() {
                return Err("The speaker model could not read that clip.".to_string());
            }
            (self.accept)(stream, SAMPLE_RATE, samples.as_ptr(), samples.len() as i32);
            (self.input_finished)(stream);
            if (self.is_ready)(self.handle, stream) == 0 {
                (self.destroy_stream)(stream);
                return Err("That clip was too short to identify a voice.".to_string());
            }
            let raw = (self.compute)(self.handle, stream);
            let length = (self.dim)(self.handle).max(0) as usize;
            let vector = if raw.is_null() || length == 0 {
                Vec::new()
            } else {
                std::slice::from_raw_parts(raw, length).to_vec()
            };
            if !raw.is_null() {
                (self.destroy_embedding)(raw);
            }
            (self.destroy_stream)(stream);
            if vector.is_empty() {
                return Err("The speaker model returned nothing.".to_string());
            }
            Ok(vector)
        }
    }
}

impl Drop for Speaker {
    fn drop(&mut self) {
        unsafe { (self.destroy)(self.handle) }
    }
}

fn load_library(path: &Path) -> Result<Library, String> {
    #[cfg(windows)]
    {
        // `LOAD_WITH_ALTERED_SEARCH_PATH` (0x8): resolve the library's own
        // dependencies from the directory it was found in, so `onnxruntime.dll`
        // is picked up beside it.
        const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 0x0000_0008;
        let loaded = unsafe {
            libloading::os::windows::Library::load_with_flags(path, LOAD_WITH_ALTERED_SEARCH_PATH)
        };
        return loaded
            .map(Library::from)
            .map_err(|error| format!("The voice recognition engine could not be loaded: {error}"));
    }
    #[cfg(not(windows))]
    {
        unsafe { Library::new(path) }
            .map_err(|error| format!("The voice recognition engine could not be loaded: {error}"))
    }
}

/// Copies one function pointer out of the library.
///
/// The pointers are plain `extern "C"` function pointers, so they are valid for
/// as long as the library lives -- which is as long as the struct holding them.
unsafe fn symbol<T: Copy>(library: &Library, name: &str) -> Result<T, String> {
    let found = library
        .get::<T>(name.as_bytes())
        .map_err(|error| format!("The engine is missing {name}: {error}"))?;
    Ok(*found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(vector: &[f32]) -> Vec<f32> {
        let length = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
        vector.iter().map(|value| value / length).collect()
    }

    /// Reads a 16-bit mono WAV's samples, which is all the enrollment path has.
    fn read_wav(path: &str) -> Vec<f32> {
        let bytes = std::fs::read(path).expect("the sample wav");
        let start = bytes
            .windows(4)
            .position(|window| window == b"data")
            .expect("a data chunk")
            + 8;
        bytes[start..]
            .chunks_exact(2)
            .map(|sample| i16::from_le_bytes([sample[0], sample[1]]) as f32 / 32_768.0)
            .collect()
    }

    /// The one test that proves the FFI layout is right: the library loads, the
    /// extractor accepts 16 kHz audio, and an embedding comes back with the
    /// dimension the model publishes.
    ///
    /// Ignored because it needs the optional engine installed:
    ///
    ///     SHERPA_DIR=<engine lib folder> SHERPA_MODEL=<model.onnx> \
    ///     SHERPA_WAV=<16 kHz mono wav> \
    ///     cargo test --lib an_embedding_comes_back -- --ignored --nocapture
    #[test]
    #[ignore = "needs the sherpa-onnx engine and a 16 kHz wav"]
    fn an_embedding_comes_back() {
        let directory = std::env::var("SHERPA_DIR").expect("SHERPA_DIR");
        let model = std::env::var("SHERPA_MODEL").expect("SHERPA_MODEL");
        let wav = std::env::var("SHERPA_WAV").expect("SHERPA_WAV");

        let speaker =
            Speaker::open(Path::new(&directory), Path::new(&model)).expect("the engine opens");
        let audio = read_wav(&wav);
        assert!(
            audio.len() > SAMPLE_RATE as usize / 2,
            "the clip is long enough"
        );
        let embedding = speaker.embed(&audio).expect("an embedding");
        eprintln!("embedding dimension: {}", embedding.len());
        assert!(!embedding.is_empty(), "the model returns a vector");
        assert!(
            embedding.iter().all(|value| value.is_finite()),
            "no NaN in the vector"
        );
        // The same clip twice must land on the same vector: a model that answers
        // differently for identical audio could never be compared with anything.
        let again = speaker.embed(&audio).expect("a second embedding");
        let drift = embedding
            .iter()
            .zip(&again)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(
            drift < 1e-4,
            "the same clip embeds the same way (drift {drift})"
        );

        // Optional: a second clip to see the scale a threshold lives on. Set it
        // to another voice and the number is what "different person" looks like
        // here, which is what the default floor was chosen against.
        if let Ok(other) = std::env::var("SHERPA_WAV_OTHER") {
            let second = speaker
                .embed(&read_wav(&other))
                .expect("a second embedding");
            // Both unit length, or the number is a scaled dot rather than a
            // cosine -- which is exactly the bug this print first caught.
            let similarity: f32 = unit(&embedding)
                .iter()
                .zip(unit(&second))
                .map(|(a, b)| a * b)
                .sum();
            eprintln!("cosine against SHERPA_WAV_OTHER: {similarity:.3}");
        }
    }
}
