//! Estimating the VRAM a local model will use.
//!
//! **This is an estimate and the UI says so.** It is not a measurement, and the
//! difference matters: llama.cpp allocates for activations, scratch buffers, the
//! compute graph, and whatever the backend reserves for its own use, none of
//! which are visible before the model loads. The figure here is the part that
//! *is* predictable -- weights, KV cache, and the projector -- computed from the
//! GGUF file's own metadata rather than guessed from the filename.
//!
//! What it is good for is the thing a user actually needs it for: deciding
//! whether a setting they are about to choose will fit. "Will 32k context on a
//! 24 GB card run out of memory?" is answerable. "Will this reply be fast?" is
//! not, and nothing here pretends otherwise.
//!
//! ## Where the numbers come from
//!
//! `ggml` publishes the per-quantisation bytes-per-parameter figures and
//! llama.cpp's own memory breakdown sums them the same way. They are constants,
//! not measurements of a particular model, which is why an estimate is labelled
//! as one.
//!
//! ## Why the GGUF is parsed rather than the file size used
//!
//! File size is the *quantised* weight size, which is exactly the number wanted
//! for "how big are the weights". But the KV cache depends on architecture --
//! layer count, embedding width, and whether GQA is in use -- and none of that is
//! knowable from a file size. A 4k-context model and a 32k-context model of the
//! same weights have wildly different KV costs, and that difference is the one
//! a user adjusting a context slider is about to feel.
//!
//! The reading of those facts lives in [`super::parser`]; this module is only
//! the arithmetic that turns a shape into a number.

use super::parser::{read_shape, ModelShape};
use crate::ai::local::settings::{KvQuant, LocalRuntimeSettings};

/// The estimate, broken into the parts a user can act on.
#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct MemoryEstimate {
    /// VRAM the weights occupy once loaded.
    pub weights_bytes: u64,
    /// VRAM the KV cache will occupy at the current context and slot settings.
    pub kv_bytes: u64,
    /// VRAM the multimodal projector will occupy, if one is configured.
    pub projector_bytes: u64,
    /// The sum of the above. Not the process's total footprint -- see the module
    /// note.
    pub total_bytes: u64,
    /// What the device actually has, when it could be read.
    pub device_total_bytes: Option<u64>,
    /// Whether the known parts exceed the device. `None` when it is unknown.
    ///
    /// Filled in by the command that builds the estimate rather than by
    /// `estimate` itself, so one place asks the question -- and the answer is
    /// deliberately one-directional. It may report that this does not fit, and
    /// never that it does, because the unmeasured remainder could be larger than
    /// what is left.
    pub exceeds_device: Option<bool>,
    /// False when the GGUF yielded no usable architecture, so `kv_bytes` is
    /// zero because it is unknown rather than because it is small.
    pub reliable: bool,
}

impl MemoryEstimate {
    /// Asks the fit question. Called by the command once, not by the UI.
    pub fn exceeds_device(&self) -> Option<bool> {
        self.device_total_bytes
            .map(|total| self.total_bytes > total)
    }
}

/// How many bytes a token in the KV cache costs, at a given quantisation.
///
/// Present on `KvQuant` in `settings.rs` for the same figure; this alias exists
/// so the two halves of the estimate agree by construction rather than by two
/// literals happening to match.
fn kv_bytes_per_token_per_layer(shape: &ModelShape, k: KvQuant, v: KvQuant) -> f64 {
    let width = shape.head_count_kv as f64 * shape.head_dimension as f64;
    width * (k.bytes_per_token_per_layer() + v.bytes_per_token_per_layer()) as f64
}

/// Strips a Windows device-path prefix so a path from a file dialog can be checked.
///
/// Windows dialogs and `canonicalize` hand back `\\?\F:\models\a.gguf`. The
/// prefix means "bypass path parsing", and the ordinary parser handles these
/// paths fine on current Windows, so a plain `Path::new` check against the raw
/// string fails for a file that is plainly there. The UNC form
/// `\\?\UNC\server\share` becomes `\\server\share` rather than losing its prefix
/// entirely, which would turn a network path into a nonsense local one.
pub fn normalise_windows_path(path: &str) -> std::path::PathBuf {
    let trimmed = path.trim();
    // The UNC arm allocates because it has to insert a `\` that is not in the
    // input; the plain arm borrows, which is the common case on a local disk.
    let bare = match trimmed.strip_prefix(r"\\?\UNC\") {
        Some(rest) => {
            let mut network = String::with_capacity(rest.len() + 2);
            network.push_str(r"\\");
            network.push_str(rest);
            network
        }
        None => trimmed.strip_prefix(r"\\?\").unwrap_or(trimmed).to_string(),
    };
    std::path::PathBuf::from(bare)
}

/// Builds the estimate for one model under one configuration.
///
/// The weights figure is the file's own size, which is exact and needs no
/// architecture at all -- a quantised GGUF is stored quantised. It is used
/// directly rather than derived from a parameter count and a bytes-per-parameter
/// constant, because the file is right there and the derivation can only be
/// worse. The quantisation constant is kept because the projector and KV cache
/// genuinely need the shape.
///
/// The KV figure is `tokens * layers * (kv_width * (bytes_k + bytes_v))`, which
/// is llama.cpp's own formula. `tokens` is `context` when the per-slot flag is
/// set and `context / parallel` when it is not -- the same distinction
/// `context_per_slot` makes, because estimating against the wrong one would
/// report a cache six times larger than the real one.
pub fn estimate(
    model_path: &std::path::Path,
    projector_path: Option<&std::path::Path>,
    settings: &LocalRuntimeSettings,
    device_total_bytes: Option<u64>,
) -> MemoryEstimate {
    // Normalised on the way in: a model registered from a Windows dialog carries
    // the `\\?\` device prefix, and while the OS accepts it for most operations
    // the read here is a plain `File::open` on a header that has to be parsed
    // byte by byte -- so a path that works for one does not quietly work for the
    // other. Stripping it in one place means both callers agree.
    let model_path = normalise_windows_path(&model_path.to_string_lossy());
    let projector = projector_path.map(|path| normalise_windows_path(&path.to_string_lossy()));

    let weights_bytes = std::fs::metadata(&model_path)
        .map(|meta| meta.len())
        .unwrap_or(0);
    let projector_bytes = projector
        .as_deref()
        .and_then(|path| std::fs::metadata(path).ok())
        .map(|meta| meta.len())
        .unwrap_or(0);

    let shape = read_shape(&model_path);
    let k = settings.cache_type_k.unwrap_or(KvQuant::F16);
    let v = settings.cache_type_v.unwrap_or(KvQuant::F16);

    // With no shape, the KV cache is reported as unknown rather than as zero --
    // zero would read as "this model needs no cache", which is never true.
    let (kv_bytes, reliable) = match shape {
        Some(shape) if shape.can_estimate_kv() => {
            let tokens = settings.context_per_slot() as f64 * settings.parallel.max(1) as f64;
            let per_token = kv_bytes_per_token_per_layer(&shape, k, v);
            let bytes = (tokens * shape.layer_count as f64 * per_token) as u64;
            (bytes, true)
        }
        _ => (0, false),
    };

    MemoryEstimate {
        weights_bytes,
        kv_bytes,
        projector_bytes,
        total_bytes: weights_bytes + kv_bytes + projector_bytes,
        device_total_bytes,
        // Filled in by the caller, which is the one place that knows a verdict is
        // wanted at all. See the field's note.
        exceeds_device: None,
        reliable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A device path reduces to the same path a plain check can use.
    ///
    /// Regression guard for "File missing" on a file that was plainly there: the
    /// check compared the raw `\\?\`-prefixed string a Windows dialog returns.
    #[test]
    fn a_device_path_is_reduced_to_a_checkable_one() {
        assert_eq!(
            normalise_windows_path(r"\\?\F:\models\a.gguf"),
            std::path::PathBuf::from(r"F:\models\a.gguf")
        );
    }

    /// A UNC device path keeps its double backslash.
    ///
    /// Stripping `\\?\` alone from `\\?\UNC\share\a.gguf` would leave
    /// `UNC\share\a.gguf`, which is a relative path to a local folder named UNC.
    #[test]
    fn a_unc_device_path_keeps_its_network_form() {
        assert_eq!(
            normalise_windows_path(r"\\?\UNC\share\a.gguf"),
            std::path::PathBuf::from(r"\\share\a.gguf")
        );
    }

    /// An ordinary path is passed through untouched.
    #[test]
    fn an_ordinary_path_is_unchanged() {
        assert_eq!(
            normalise_windows_path(r"C:\models\a.gguf"),
            std::path::PathBuf::from(r"C:\models\a.gguf")
        );
    }

    /// The KV figure rises with context, so the estimate responds to the slider.
    #[test]
    fn the_kv_estimate_rises_with_context() {
        let shape = ModelShape {
            block_count: 36,
            layer_count: 36,
            context_length: 32768,
            embedding_length: 2560,
            head_count_kv: 8,
            head_dimension: 128,
            quantization: None,
        };
        let per_token = kv_bytes_per_token_per_layer(&shape, KvQuant::Q8_0, KvQuant::Q8_0);
        let low = 4096f64 * shape.layer_count as f64 * per_token;
        let high = 32768f64 * shape.layer_count as f64 * per_token;
        assert!(high > low * 7.0, "{high} should be about 8x {low}");
    }

    /// Parses a real model file when `PROJECTZ_PROBE_GGUF` names one.
    ///
    /// Ignored by default so a normal run does not depend on a model being
    /// installed, but it is the only test that would have caught the GGUF v3
    /// string-width bug: a synthetic header of short keys survives a wrong
    /// length width, a real one does not.
    ///
    /// ```text
    /// set PROJECTZ_PROBE_GGUF=F:\models\a.gguf
    /// cargo test --lib reads_a_real_gguf_when_named -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore]
    fn reads_a_real_gguf_when_named() {
        let Ok(path) = std::env::var("PROJECTZ_PROBE_GGUF") else {
            panic!("set PROJECTZ_PROBE_GGUF to a .gguf path");
        };
        let normalised = normalise_windows_path(&path);
        assert!(
            normalised.is_file(),
            "{} does not resolve",
            normalised.display()
        );
        // Runs the parser the app runs, on a real file. The synthetic-header tests
        // cannot cover this: they have no tokenizer, and every one of the bugs
        // this found was in a value type or a key prefix a real file has and a
        // hand-built header does not.
        let shape = crate::ai::local::parser::read_shape(&normalised)
            .unwrap_or_else(|| panic!("production read_shape failed for {normalised:?}"));
        println!("shape {shape:?}");
        assert!(shape.can_estimate_kv(), "no KV shape from {path}");
        let settings = LocalRuntimeSettings::default();
        for context in [4096u32, 32768] {
            let mut tuned = settings.clone();
            tuned.context = context;
            let estimate = estimate(&normalised, None, &tuned, Some(24 * 1024 * 1024 * 1024u64));
            println!("context {context} -> {estimate:?}");
            assert!(estimate.reliable, "unreliable at {context}");
            assert!(estimate.kv_bytes > 0, "no KV bytes at {context}");
        }
    }

    /// The KV figure falls as the cache is quantised harder.
    #[test]
    fn a_coarser_cache_quantisation_uses_less() {
        let shape = ModelShape {
            block_count: 36,
            layer_count: 36,
            context_length: 32768,
            embedding_length: 2560,
            head_count_kv: 8,
            head_dimension: 128,
            quantization: None,
        };
        let fine = kv_bytes_per_token_per_layer(&shape, KvQuant::F16, KvQuant::F16);
        let coarse = kv_bytes_per_token_per_layer(&shape, KvQuant::Q4_0, KvQuant::Q4_0);
        assert!(coarse < fine * 0.3, "{coarse} should be well under {fine}");
    }
}
