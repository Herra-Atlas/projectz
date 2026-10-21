//! What each kind of download is, and where it lands.
//!
//! One place per rule, so adding recognition later is one enum arm rather than a
//! third copy of "which folder, which files, which sidecars" spread across the
//! page, the installer and the reader.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::app_paths::models_root;

/// A kind of thing the downloader knows how to install.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A GGUF model run by llama.cpp.
    Llama,
    /// A Piper voice: an `.onnx` model and the `.onnx.json` beside it.
    Tts,
    /// A Whisper model: one `ggml-*.bin` file.
    Stt,
}

impl Kind {
    /// The folder under `models/` this kind installs into.
    pub fn folder(self) -> &'static str {
        match self {
            Self::Llama => "llama",
            Self::Tts => "tts",
            Self::Stt => "stt",
        }
    }

    /// Where one repository installs.
    ///
    /// A repository name contains a slash, which would make it read as two
    /// folders -- so the two halves are joined with `__`, which no repository name
    /// can contain. Two repositories therefore cannot collide by accident.
    pub fn install_dir(self, repo: &str) -> PathBuf {
        models_root_for(self).join(repo.replace('/', "__"))
    }

    /// The path inside `repo` this file is written to.
    ///
    /// llama.cpp keeps the repository's own layout, because the files beside a
    /// model belong to it: a projector (`mmproj`) or a tokenizer downloaded later
    /// then lands where the loader already looks.
    ///
    /// A Piper voice is flattened to its file name. The index nests it four
    /// folders deep (`en/en_US/amy/low/...`), and the name already carries the
    /// language, the voice and the quality -- the folders would only repeat it.
    pub fn destination(self, repo: &str, path: &str) -> PathBuf {
        let relative = match self {
            Self::Llama => PathBuf::from(path),
            // A voice and a Whisper model are named for themselves: the repository
            // path is a packaging detail, and a reader looking for one should not
            // have to walk the folders it happens to sit in.
            Self::Tts | Self::Stt => PathBuf::from(file_name(path)),
        };
        self.install_dir(repo).join(relative)
    }

    /// Every file that has to be fetched for `path` to be usable.
    ///
    /// A voice without its `.onnx.json` cannot be loaded -- that is where Piper
    /// reads the sample rate and the phoneme table -- so the sidecar is not an
    /// optional extra to fetch later. A Whisper model is one file.
    pub fn companions(self, path: &str) -> Vec<String> {
        match self {
            Self::Llama | Self::Stt => vec![path.to_string()],
            Self::Tts => vec![path.to_string(), format!("{path}.json")],
        }
    }
}

pub fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// `models/<kind>`, the root every install for a kind sits under.
pub fn models_root_for(kind: Kind) -> PathBuf {
    models_root().join(kind.folder())
}

/// The quantisation a GGUF's file name states, for the row's note.
/// A filename is the only place this is written down before the header is read,
/// which would cost a partial download. Unknown is left empty rather than guessed.
pub fn quantisation(path: &str) -> Option<String> {
    const QUANTS: [&str; 24] = [
        "IQ1_S", "IQ2_XXS", "IQ2_XS", "IQ2_S", "IQ2_M", "IQ3_XXS", "IQ3_XS", "IQ3_S", "IQ3_M",
        "IQ4_XS", "IQ4_NL", "Q2_K", "Q2_K_S", "Q3_K_S", "Q3_K_M", "Q3_K_L", "Q4_0", "Q4_1",
        "Q4_K_S", "Q4_K_M", "Q5_0", "Q5_1", "Q5_K_S", "Q5_K_M",
    ];
    let upper = path.to_ascii_uppercase();
    let mut found: Option<(usize, &str)> = None;
    for quant in QUANTS {
        if let Some(at) = upper.find(quant) {
            // The longest match wins: `Q2_K_S` contains `Q2_K`, and the file says
            // which one it is.
            if found.is_none_or(|(_, best)| quant.len() > best.len()) {
                found = Some((at, quant));
            }
        }
    }
    if let Some((_, quant)) = found {
        return Some(quant.to_string());
    }
    for (name, quant) in [("F16", "F16"), ("BF16", "BF16"), ("F32", "F32")] {
        if upper.contains(name) {
            return Some(quant.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn a_repository_slash_becomes_one_folder() {
        let directory = Kind::Tts.install_dir("rhasspy/piper-voices");
        assert_eq!(
            directory.file_name().and_then(|name| name.to_str()),
            Some("rhasspy__piper-voices")
        );
        assert_eq!(directory.parent().and_then(|path| path.file_name()).and_then(|name| name.to_str()), Some("tts"));
    }

    #[test]
    fn a_voice_is_flattened_and_keeps_its_settings_beside_it() {
        let path = "en/en_US/amy/low/en_US-amy-low.onnx";
        let destination = Kind::Tts.destination("rhasspy/piper-voices", path);
        assert_eq!(
            destination.file_name().and_then(|name| name.to_str()),
            Some("en_US-amy-low.onnx")
        );
        assert_eq!(
            Kind::Tts.companions(path),
            vec![path.to_string(), format!("{path}.json")]
        );
    }

    #[test]
    fn a_gguf_keeps_the_repository_layout_so_a_projector_lands_beside_it() {
        let destination = Kind::Llama.destination("acme/tiny", "nested/tiny.gguf");
        assert!(destination.ends_with(Path::new("nested").join("tiny.gguf")));
        assert_eq!(Kind::Llama.companions("nested/tiny.gguf"), vec!["nested/tiny.gguf"]);
    }
}
