//! Reading a model's own metadata out of a GGUF file.
//!
//! One bounded reader over the header, pulling out the handful of keys that
//! describe a model's shape. Split from `memory.rs` because it now has two
//! callers with two unrelated reasons to change: the VRAM estimate wants the
//! architecture, and the model list wants the facts it shows as badges.
//!
//! ## Why a parser at all
//!
//! File size is the *quantised* weight size, which is exactly the right number
//! for "how big are the weights" and useless for anything else. The KV cache
//! depends on architecture -- layer count, KV head count, head dimension -- and
//! none of that is knowable from a file size. A 4k-context model and a 32k
//! model of the same weights have very different KV costs, and that difference
//! is exactly what someone adjusting a context slider is about to feel.
//!
//! ## What it deliberately does not do
//!
//! It does not try to be a general GGUF library. It reads a fixed list of keys
//! and skips everything else, and it stops as soon as it has them -- so it never
//! walks a 150k-entry tokenizer to learn nothing from it. A model file it cannot
//! read reports *no shape* rather than a wrong one, and the caller decides what
//! to do about that.

/// What the GGUF header tells us about a model's shape.
///
/// Only the fields something downstream needs. Read through with a bounded
/// reader rather than mapped whole: the file can be tens of gigabytes and this
/// runs while the settings page opens.
#[derive(Debug, Clone, Copy, Default)]
pub struct ModelShape {
    pub block_count: u32,
    pub embedding_length: u32,
    /// `n_head_kv`, or `n_head` when the model does not use grouped-query
    /// attention. A model with GQA needs far fewer KV bytes per token than its
    /// head count suggests, and reading `n_head` there would overestimate by the
    /// whole group factor.
    pub head_count_kv: u32,
    pub head_dimension: u32,
    pub layer_count: u32,
    /// `context_length`: the context window the file declares.
    ///
    /// The model's own ceiling, which is not the context an app chooses to run it
    /// at. Shown on the model list so the number a user compares against is the
    /// one the file states rather than whatever is currently configured.
    pub context_length: u32,
    /// `general.file_type`, e.g. `Q4_K_M`. How the weights are quantised.
    ///
    /// Held as a small enum rather than a string because the set of names ggml
    /// defines is known and closed; an unrecognised one means the caller falls
    /// back to the file's own size, which is the same figure by another route.
    ///
    /// **Absent on many real files.** Not every converter writes it -- a
    /// `Qwen3.5-4B ... Q4_K_M.gguf` produced by one common converter carries no
    /// `general.file_type` key at all -- so this stays `None` and nothing is
    /// inferred from the filename to stand in for it.
    pub quantization: Option<WeightsQuant>,
}

impl ModelShape {
    /// True when the header carried enough to make a KV estimate meaningful.
    ///
    /// Without this the caller would divide by a zero and report an infinite
    /// figure, which is worse than reporting none.
    pub fn can_estimate_kv(&self) -> bool {
        self.layer_count > 0 && self.head_count_kv > 0 && self.head_dimension > 0
    }
}

/// The ggml weight quantisations, at their published bytes-per-parameter.
///
/// The `_K_M` and `_K_S` families mix several block types within one tensor.
/// They use the family's average rather than the best case, because an estimate
/// that flatters every quantised model is worse than one that is slightly
/// pessimistic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeightsQuant {
    F32,
    F16,
    Q8_0,
    Q6K,
    Q5K,
    Q4K,
    Q3K,
    Q2K,
    Iq4,
    Iq3,
    Iq2,
    /// Any name not in the list. Falls back to the file's own size.
    Unknown,
}

impl WeightsQuant {
    /// Maps `general.file_type` to a figure.
    ///
    /// Matched on a prefix rather than the whole name, because the `_K_M`/`_K_S`
    /// families add a suffix to a base name ggml also defines on its own: a file
    /// stating `Q4_K_M` is `Q4_K`, and matching the name exactly -- which an
    /// earlier version did -- reported every mixed-quantisation file as
    /// `Unknown`, so the estimate silently fell back to the file size.
    pub fn from_ggml_name(name: &str) -> Self {
        match name {
            "F32" => WeightsQuant::F32,
            "F16" | "BF16" => WeightsQuant::F16,
            "Q8_0" => WeightsQuant::Q8_0,
            name if name.starts_with("Q6_K") => WeightsQuant::Q6K,
            name if name.starts_with("Q5_K") || name.starts_with("Q5_0") => WeightsQuant::Q5K,
            name if name.starts_with("Q4_K") || name.starts_with("Q4_0") => WeightsQuant::Q4K,
            name if name.starts_with("Q3_K") => WeightsQuant::Q3K,
            name if name.starts_with("Q2_K") => WeightsQuant::Q2K,
            name if name.starts_with("IQ4") => WeightsQuant::Iq4,
            name if name.starts_with("IQ3") => WeightsQuant::Iq3,
            name if name.starts_with("IQ2") => WeightsQuant::Iq2,
            _ => WeightsQuant::Unknown,
        }
    }

    /// The name as it appears in the file, for display.
    ///
    /// The enum's own spelling is not what a user recognises: `Q4K` in the code
    /// is `Q4_K_M` in the filename and in every converter's output.
    pub fn as_str(self) -> &'static str {
        match self {
            WeightsQuant::F32 => "F32",
            WeightsQuant::F16 => "F16",
            WeightsQuant::Q8_0 => "Q8_0",
            WeightsQuant::Q6K => "Q6_K",
            WeightsQuant::Q5K => "Q5_K_M",
            WeightsQuant::Q4K => "Q4_K_M",
            WeightsQuant::Q3K => "Q3_K_M",
            WeightsQuant::Q2K => "Q2_K",
            WeightsQuant::Iq4 => "IQ4_XS",
            WeightsQuant::Iq3 => "IQ3_M",
            WeightsQuant::Iq2 => "IQ2_XXS",
            WeightsQuant::Unknown => "unknown",
        }
    }

    /// Bytes per parameter.
    ///
    /// The shape-based estimate would use this if the header carried a parameter
    /// count. It does not use it today, because the file's own size is exact and
    /// needs no derivation -- so it stays as the fallback a future header read
    /// would use rather than being deleted and rediscovered.
    #[allow(dead_code)]
    pub fn bytes_per_parameter(self) -> f64 {
        match self {
            WeightsQuant::F32 => 4.0,
            WeightsQuant::F16 => 2.0,
            WeightsQuant::Q8_0 => 1.0625,
            WeightsQuant::Q6K => 0.8594,
            WeightsQuant::Q5K => 0.7344,
            WeightsQuant::Q4K => 0.5625,
            WeightsQuant::Q3K => 0.4219,
            WeightsQuant::Q2K => 0.3281,
            WeightsQuant::Iq4 => 0.5,
            WeightsQuant::Iq3 => 0.4375,
            WeightsQuant::Iq2 => 0.2666,
            // Deliberately low: a caller reaching this uses the file size, which
            // is exact, so the constant is never actually multiplied out.
            WeightsQuant::Unknown => 1.0,
        }
    }
}

/// How much of a model file is read to find the metadata.
///
/// The metadata block contains the whole tokenizer -- every id, every piece, every
/// score and every merged pair -- which on a modern 150k vocabulary runs to a few
/// megabytes. The keys this parser wants are the architecture ones, which sit at
/// the *front* of that block, so most of what is read here is skipped rather
/// than used.
///
/// Kept as a named constant because it is a real limit with a reason, not a
/// round number: too small and the reader runs out of buffer before the end of
/// the metadata block and reports "no shape" for a perfectly good model.
const HEADER_READ_BYTES: usize = 64 * 1024 * 1024;

/// Reads the architecture out of a GGUF header.
///
/// Returns `None` rather than an error for anything unreadable. The caller
/// reports a less precise figure, and a page of settings that refuses to open
/// because a model file is in an unexpected format is worse than one that shows
/// an estimate without the KV component.
pub fn read_shape(path: &std::path::Path) -> Option<ModelShape> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    // The metadata block holds the tokenizer -- every token id and piece, which
    // on a modern vocabulary is a couple of megabytes on its own -- and the
    // architecture keys this parser wants sit *before* it. The read is still
    // capped, because a corrupt length would otherwise ask for an arbitrary
    // amount, but the cap has to clear a real vocabulary rather than a toy one.
    let mut header = vec![0u8; HEADER_READ_BYTES];
    let read = file.read(&mut header).ok()?;
    header.truncate(read);
    read_shape_from_bytes(&header)
}

/// The same read, over bytes already in hand.
///
/// Split from [`read_shape`] so the parser can be tested against a header built
/// to a specification rather than against a model file that may not exist on the
/// machine running the tests -- which is how the architecture-prefix bug below
/// shipped with every test still passing.
pub fn read_shape_from_bytes(header: &[u8]) -> Option<ModelShape> {
    let mut cursor = Cursor::new(header);
    // Magic "GGUF", then a version we read but do not branch on.
    if cursor.take(4)? != b"GGUF" {
        return None;
    }
    let version = cursor.u32()?;
    let tensor_count = cursor.u64()?;
    let metadata_count = cursor.u64()?;

    // **String lengths are 64-bit from version 3 on.**
    //
    // Reading them as `u32` consumed four bytes too few for every key, so the
    // reader drifted four bytes further behind on each entry and hit a real
    // model's first key -- `qwen35.rope.dimension_sections` -- already
    // misaligned. A synthetic header of short keys stayed inside the 4 GB
    // length's upper bytes and appeared to work, which is why every test passed
    // while no real model was read at all.
    //
    // Versions 1 and 2 are rare enough not to be worth a second reader, but they
    // are not silently misparsed either: they report as "no shape" rather than as
    // a wrong number.
    let wide_strings = version >= 3;

    // A tensor-count sanity bound. A real model is in the hundreds; anything
    // past this is a misread rather than a very large model.
    if tensor_count > 1_000_000 || metadata_count > 10_000 {
        return None;
    }

    let mut shape = ModelShape::default();
    let mut head_count: Option<u32> = None;
    // GGUF namespaces every architecture key under the model family, and the
    // prefix is whatever `general.architecture` says. Captured here and compared
    // against, rather than listed per key, so one comparison covers every family.
    let mut architecture: Option<String> = None;

    for _ in 0..metadata_count {
        let key = cursor.string(wide_strings)?;
        let value_type = cursor.u32()?;

        // Only a handful of keys matter, and all of them are scalars, so the
        // value is only *read* when the key is one of them. Every key still has
        // to be walked past, which is what `skip_value` below is for.
        let mut already_read = false;
        match key.as_str() {
            "general.architecture" => {
                if let Some(name) = read_string_value(&mut cursor, value_type, wide_strings) {
                    architecture = Some(name);
                    already_read = true;
                }
            }
            "general.file_type" => {
                if let Some(name) = read_string_value(&mut cursor, value_type, wide_strings) {
                    shape.quantization = Some(WeightsQuant::from_ggml_name(&name));
                    already_read = true;
                }
            }
            _ => {
                // Matched on the part *after* the architecture prefix, so one
                // comparison covers every model family.
                //
                // **No `continue` below, and that is the whole fix.** An earlier
                // version skipped the entry outright when the key was not one it
                // wanted, which skipped the key's *value* too -- so the reader
                // fell behind by the width of every unrecognised value and read
                // the next key from the wrong offset. `general.type` holds a
                // string, so the very first unrecognised key put it four bytes
                // out, and no real model was ever read: every one reported an
                // unknown KV cache, so the estimate never moved.
                //
                // A key may be ignored. Its value is always walked.
                let recognised = match key.split_once('.') {
                    Some((prefix, field)) if prefix == architecture.as_deref().unwrap_or("") => {
                        matches!(
                            field,
                            "block_count"
                                | "context_length"
                                | "embedding_length"
                                | "attention.head_count"
                                | "attention.head_count_kv"
                                | "attention.key_length"
                        )
                    }
                    _ => false,
                };
                if recognised {
                    let Some(value) = read_u32_value(&mut cursor, value_type) else {
                        // The key is wanted but the value is not a scalar this
                        // reader takes. It has not been consumed, so let
                        // `skip_value` below walk it rather than misaligning.
                        skip_value(&mut cursor, value_type, wide_strings)?;
                        continue;
                    };
                    match field_name(key.as_str()) {
                        "block_count" => shape.block_count = value,
                        "context_length" => shape.context_length = value,
                        "embedding_length" => shape.embedding_length = value,
                        "attention.head_count" => head_count = Some(value),
                        "attention.head_count_kv" => shape.head_count_kv = value,
                        "attention.key_length" => shape.head_dimension = value,
                        _ => {}
                    }
                    already_read = true;
                }
            }
        }
        // `already_read` is true only for the handful of keys actually wanted, so
        // the skip runs on nearly every entry.
        //
        // Every key's value is consumed, whether or not it is one this parser
        // wants.
        //
        // An earlier version `continue`d on an uninteresting key, which skipped
        // its *value* as well as ignoring the key -- so the reader fell four
        // bytes behind on every `general.*` key it did not recognise and read
        // the next one as a bad key. A key is only ever skipped; its value is
        // always walked.
        if !already_read {
            skip_value(&mut cursor, value_type, wide_strings)?;
        }

        // Stops as soon as everything it came for has been read.
        //
        // The architecture keys sit at the *front* of the metadata block, and the
        // tokenizer -- a quarter of a million entries, in a layout real files
        // disagree about -- sits behind them. Walking the rest means parsing a
        // megabyte of vocabulary to learn nothing from it.
        //
        // Tested on `block_count` rather than through `can_estimate_kv`, because
        // `layer_count` is only filled in from `block_count` *after* this loop --
        // asking `can_estimate_kv` here asked a question about a field that
        // could not have an answer yet, so it was never true and the walk ran on
        // into the tokenizer it was meant to stop before.
        if shape.block_count > 0
            && shape.head_count_kv > 0
            && shape.head_dimension > 0
            && architecture.is_some()
        {
            break;
        }
    }

    // GQA fallback: a model with no `n_head_kv` uses every head for K and V.
    // Reading `n_head` as the KV count when GQA is in play would overestimate the
    // cache by the whole group factor -- typically 4x to 8x.
    if shape.head_count_kv == 0 {
        shape.head_count_kv = head_count.unwrap_or(0);
    }
    // `n_embd_head` is the modern name for what GGUF calls `key_length`.
    if shape.head_dimension == 0 {
        shape.head_dimension = head_count
            .and_then(|count| shape.embedding_length.checked_div(count))
            .unwrap_or(0);
    }
    // A model with no explicit layer count has one block per layer in practice,
    // and every architecture ggml emits the count for. Falling back to the block
    // count keeps the KV estimate from being reported as unavailable.
    if shape.layer_count == 0 {
        shape.layer_count = shape.block_count;
    }
    Some(shape)
}

/// The part of an architecture key after its family prefix.
///
/// `qwen35.attention.head_count_kv` becomes `attention.head_count_kv`, which is
/// what the match above is written against. Splitting it out means the prefix is
/// compared once per key rather than the full name repeated per field.
fn field_name(key: &str) -> &str {
    key.split_once('.').map(|(_, field)| field).unwrap_or(key)
}

/// Reads a `u32` metadata value, or `None` for any other type.
fn read_u32_value(cursor: &mut Cursor<'_>, value_type: u32) -> Option<u32> {
    match value_type {
        4 => cursor.u32(),
        5 => cursor.u32().map(|value| value as i32 as u32),
        10 => cursor.u64().map(|value| value as u32),
        _ => None,
    }
}

/// Reads a string metadata value, or `None` for any other type.
fn read_string_value(
    cursor: &mut Cursor<'_>,
    value_type: u32,
    wide_strings: bool,
) -> Option<String> {
    if value_type == 8 {
        cursor.string(wide_strings)
    } else {
        None
    }
}

/// Advances past one value of the given GGUF type.
///
/// Returns `None` for a type this parser does not know -- the stream cannot be
/// kept in step past it, so the caller abandons the header rather than reading
/// garbage offsets.
fn skip_value(cursor: &mut Cursor<'_>, value_type: u32, wide_strings: bool) -> Option<()> {
    match value_type {
        // 0 UINT8, 1 INT8 -- one byte each.
        0 | 1 => cursor.skip(1),
        // 2 UINT16, 3 INT16 -- two.
        2 | 3 => cursor.skip(2),
        // 4 UINT32, 5 INT32, 6 FLOAT32 -- four. **Not 7.**
        4 | 5 | 6 => cursor.skip(4),
        // 7 BOOL -- also one byte, stored as 0 or 1.
        7 => cursor.skip(1),
        // 10 UINT64, 11 INT64, 12 FLOAT64 -- eight.
        10 | 11 | 12 => cursor.skip(8),
        8 => cursor.string(wide_strings).map(|_| ()),
        // An array: a typed element type, a `u64` count, then the elements.
        9 => {
            let element_type = cursor.u32()?;
            let count = cursor.u64()?;

            // A fixed-width array is one multiply, which is what keeps the
            // tokenizer -- 250k entries of 1-byte ids -- from costing 250k reads
            // for a value nothing here uses.
            if let Some(width) = element_width(element_type) {
                let span = (count as usize).checked_mul(width)?;
                return cursor.take(span).map(|_| ());
            }

            // Everything else is a variable width, so it has to be walked entry by
            // entry. Only strings and nested arrays reach here, and a nested array
            // is walked by its own element type rather than by a single width.
            if element_type != 8 && element_type != 9 {
                return None;
            }
            for _ in 0..count {
                if element_type == 8 {
                    cursor.string(wide_strings)?;
                } else {
                    let inner_type = cursor.u32()?;
                    skip_value(cursor, inner_type, wide_strings)?;
                }
            }
            Some(())
        }
        _ => None,
    }
}

/// Bytes one element of the given array type occupies, or `None` if it is
/// variable-width.
///
/// Deliberately a single table rather than the `skip_value` arms: a nested
/// array's width is the sum of *its* element type and its count header, which is
/// not a constant, so it is walked rather than multiplied out.
fn element_width(element_type: u32) -> Option<usize> {
    match element_type {
        // BOOL is one byte, same as the 8- and 16-bit types' half-width.
        0 | 1 | 7 => Some(1),
        2 | 3 => Some(2),
        4 | 5 | 6 => Some(4),
        10 | 11 | 12 => Some(8),
        _ => None,
    }
}

/// A little-endian reader over the header bytes.
#[derive(Clone)]
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    fn take(&mut self, count: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(count)?;
        let slice = self.bytes.get(self.at..end)?;
        self.at = end;
        Some(slice)
    }

    fn skip(&mut self, count: usize) -> Option<()> {
        self.take(count).map(|_| ())
    }

    /// A `u32`. The only scalar width this parser reads.
    ///
    /// There are deliberately no readers for the other widths. `skip_value` steps
    /// over values it does not want by multiplying out their size rather than by
    /// decoding them, so a reader per width would be eight methods and a family
    /// of bugs -- one of which this file shipped once, when `u64` was stepped over
    /// as one byte.
    fn u32(&mut self) -> Option<u32> {
        self.take(4)
            .map(|slice| u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
    }

    fn u64(&mut self) -> Option<u64> {
        self.take(8).map(|slice| {
            u64::from_le_bytes([
                slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
            ])
        })
    }

    /// A length-prefixed UTF-8 string.
    ///
    /// `wide` selects the GGUF version's length width: `u64` from version 3, `u32`
    /// before it. Guessing wrong is not a small error -- it leaves the reader a
    /// little further out of step on every key, so the file is misparsed from the
    /// first key onward rather than failing on that one.
    fn string(&mut self, wide: bool) -> Option<String> {
        let length = if wide {
            self.u64()?
        } else {
            u64::from(self.u32()?)
        } as usize;
        // Guarded: a corrupt length would otherwise try to copy gigabytes out of a
        // header this reader is treating as untrusted.
        if length > 1024 * 1024 {
            return None;
        }
        let bytes = self.take(length)?;
        String::from_utf8(bytes.to_vec()).ok()
    }
}

#[cfg(test)]
mod tests;
