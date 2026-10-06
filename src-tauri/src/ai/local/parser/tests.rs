//! Tests for the GGUF header reader.
//!
//! Kept apart from the estimate's tests because the two answer different
//! questions, and because the headers here are built to a specification on
//! purpose -- see [`gguf_header`] for why a fixture would be the wrong input.

use super::*;

/// A minimal GGUF header, written the way a converter writes one.
///
/// Built here rather than shipped as a fixture: the parser's job is being right
/// about names it has never seen, so the input has to be constructed from the
/// specification rather than copied from whichever model happened to be on the
/// machine. A fixture would have hidden the bug this now covers.
fn gguf_header(architecture: &str, block_count: u32, kv_heads: u32, key_length: u32) -> Vec<u8> {
    fn string(bytes: &mut Vec<u8>, value: &str) {
        bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
        bytes.extend_from_slice(value.as_bytes());
    }
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"GGUF");
    bytes.extend_from_slice(&3u32.to_le_bytes());
    bytes.extend_from_slice(&0u64.to_le_bytes()); // tensor count
    bytes.extend_from_slice(&7u64.to_le_bytes()); // metadata entries
    string(&mut bytes, "general.architecture");
    bytes.extend_from_slice(&8u32.to_le_bytes());
    string(&mut bytes, architecture);
    string(&mut bytes, "general.file_type");
    bytes.extend_from_slice(&8u32.to_le_bytes());
    string(&mut bytes, "Q4_K_M");
    for (field, value) in [
        ("block_count", block_count),
        ("embedding_length", 2560),
        ("attention.head_count", 32u32),
        ("attention.head_count_kv", kv_heads),
        ("attention.key_length", key_length),
    ] {
        string(&mut bytes, &format!("{architecture}.{field}"));
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

/// Architecture keys are namespaced by family, and the family varies.
///
/// The parser once matched `llama.block_count` literally, which read Llama
/// models and reported every Qwen, Gemma and Mistral model as having an unknown
/// KV cache -- so the estimate never moved and the cache column read "unknown"
/// on any model the user was likely to own.
#[test]
fn keys_are_matched_under_whichever_architecture_prefix_the_file_uses() {
    for family in ["llama", "qwen35", "gemma3", "mistral", "phi3"] {
        let shape = read_shape_from_bytes(&gguf_header(family, 36, 8, 128))
            .unwrap_or_else(|| panic!("{family} header was not read"));
        assert_eq!(shape.layer_count, 36, "{family}");
        assert_eq!(shape.head_count_kv, 8, "{family}");
        assert_eq!(shape.head_dimension, 128, "{family}");
        assert!(shape.can_estimate_kv(), "{family}");
    }
}

/// A key from another family is ignored rather than misread.
///
/// `tokenizer.ggml.model` is `gpt2` on a Qwen file, and its value is a
/// *string*, not a layer count. Reading it as one would be nonsense.
#[test]
fn a_key_from_another_family_is_not_read_as_architecture() {
    let mut bytes = gguf_header("qwen35", 36, 8, 128);
    // Append one entry under a different prefix.
    fn string(bytes: &mut Vec<u8>, value: &str) {
        bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
        bytes.extend_from_slice(value.as_bytes());
    }
    string(&mut bytes, "tokenizer.ggml.model");
    bytes.extend_from_slice(&8u32.to_le_bytes());
    string(&mut bytes, "gpt2");
    let shape = read_shape_from_bytes(&bytes).expect("header still reads");
    assert_eq!(shape.layer_count, 36);
}

/// A header that declares no quantisation reports none.
///
/// Not every converter writes `general.file_type`: a real `Qwen3.5-4B ... Q4_K_M`
/// file carries only `general.architecture`, `.type`, `.name`, `.basename` and
/// `.size_label`. So `None` here is the ordinary case for a current model, and
/// the right behaviour is to report nothing rather than to guess from the
/// filename -- which this test exists to keep honest.
#[test]
fn a_header_without_a_file_type_reports_no_quantisation() {
    fn string(bytes: &mut Vec<u8>, value: &str) {
        bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
        bytes.extend_from_slice(value.as_bytes());
    }
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"GGUF");
    bytes.extend_from_slice(&3u32.to_le_bytes());
    bytes.extend_from_slice(&0u64.to_le_bytes());
    bytes.extend_from_slice(&2u64.to_le_bytes());
    string(&mut bytes, "general.architecture");
    bytes.extend_from_slice(&8u32.to_le_bytes());
    string(&mut bytes, "qwen35");
    string(&mut bytes, "qwen35.block_count");
    bytes.extend_from_slice(&4u32.to_le_bytes());
    bytes.extend_from_slice(&32u32.to_le_bytes());

    let shape = read_shape_from_bytes(&bytes).expect("header still reads");
    assert_eq!(shape.layer_count, 32);
    assert_eq!(shape.quantization, None);
}

/// The declared quantisation is read when the file does carry one.
#[test]
fn a_declared_file_type_is_read() {
    let shape = read_shape_from_bytes(&gguf_header("qwen35", 36, 8, 128)).expect("reads");
    assert_eq!(shape.quantization, Some(WeightsQuant::Q4K));
    // Displayed as the name a user recognises, not the enum's own spelling.
    assert_eq!(shape.quantization.unwrap().as_str(), "Q4_K_M");
}

/// Something that is not a GGUF is reported as no shape rather than as garbage.
#[test]
fn a_file_that_is_not_a_gguf_is_refused() {
    assert!(read_shape_from_bytes(b"not a gguf at all").is_none());
}

/// Every scalar type is skipped at its own width, so one odd value cannot shift
/// every key after it.
///
/// **`BOOL` is the one this got wrong.** It is type 7 and occupies **one** byte,
/// but it was grouped with the 32-bit types and skipped as four. A header that
/// declared any boolean -- `kda.safe_gate` on a Ling 3.0 file, for one -- then
/// left the reader three bytes out of step, and every following key was read as
/// noise. The walk aborted partway through the metadata block and the whole model
/// reported no shape at all, so it could not be listed, estimated, or loaded.
///
/// The header below carries one value of *each* scalar type ahead of the
/// architecture keys, because the failure only shows on the types a given file
/// happens to use -- a Qwen header with no boolean in it walked fine.
#[test]
fn a_scalar_of_every_width_leaves_the_reader_in_step() {
    fn string(bytes: &mut Vec<u8>, value: &str) {
        bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
        bytes.extend_from_slice(value.as_bytes());
    }
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"GGUF");
    bytes.extend_from_slice(&3u32.to_le_bytes());
    bytes.extend_from_slice(&0u64.to_le_bytes()); // tensor count
                                                  // 1 general.architecture + 12 scalars + 5 architecture entries.
    bytes.extend_from_slice(&18u64.to_le_bytes());

    string(&mut bytes, "general.architecture");
    bytes.extend_from_slice(&8u32.to_le_bytes());
    string(&mut bytes, "qwen35");

    // One value of each fixed-width scalar type, in the order GGUF defines them.
    for (key, value_type, payload) in [
        ("general.scalar_u8", 0u32, &[1u8][..]),
        ("general.scalar_i8", 1, &[1]),
        ("general.scalar_u16", 2, &[1, 0]),
        ("general.scalar_i16", 3, &[1, 0]),
        ("general.scalar_u32", 4, &[1, 0, 0, 0]),
        ("general.scalar_i32", 5, &[1, 0, 0, 0]),
        ("general.scalar_f32", 6, &[0, 0, 128, 63]),
        ("general.bool_true", 7, &[1]),
        ("general.bool_false", 7, &[0]),
        ("general.scalar_u64", 10, &[1, 0, 0, 0, 0, 0, 0, 0]),
        ("general.scalar_i64", 11, &[1, 0, 0, 0, 0, 0, 0, 0]),
        ("general.scalar_f64", 12, &[0, 0, 0, 0, 0, 0, 240, 63]),
    ] {
        string(&mut bytes, key);
        bytes.extend_from_slice(&value_type.to_le_bytes());
        bytes.extend_from_slice(payload);
    }

    for (field, value) in [
        ("block_count", 24u32),
        ("context_length", 131_072),
        ("embedding_length", 1536),
        ("attention.head_count", 24u32),
        ("attention.head_count_kv", 16),
    ] {
        string(&mut bytes, &format!("qwen35.{field}"));
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    let shape = read_shape_from_bytes(&bytes).expect("every scalar width was walked");
    assert_eq!(shape.block_count, 24);
    assert_eq!(shape.context_length, 131_072);
    assert_eq!(shape.embedding_length, 1536);
    assert_eq!(shape.head_count_kv, 16);
    assert_eq!(shape.layer_count, 24);
}
