//! Turning HuggingFace's answers into the rows the page draws.
//!
//! The page wants one shape whatever the kind: a name, its repository, a size,
//! and a short fact that tells two files in the same repository apart. Building
//! that here, rather than in the command, is what keeps the two kinds from
//! growing two different result views.

use serde::{Deserialize, Serialize};

use super::kinds::quantisation;

/// One installable file, as the page sees it.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadFile {
    pub repo: String,
    /// The path inside the repository, which is what an install names.
    pub path: String,
    /// The file's own name, which is what a reader recognises.
    pub name: String,
    pub size: u64,
    /// Quantisation, quality or language -- whatever distinguishes this file.
    pub note: String,
    /// The section of this kind's catalogue the file belongs to, when the kind
    /// has more than one: an audio.cpp package's task, or which runtime a wake
    /// row needs. Left off the wire for the single-list kinds, so the page can
    /// hide its section filter rather than offer one choice.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

/// One entry from `/api/models/{repo}/tree/main`.
#[derive(Deserialize)]
pub struct TreeEntry {
    #[serde(rename = "type")]
    pub entry_type: String,
    pub path: String,
    #[serde(default)]
    pub size: u64,
}

/// The `.gguf` files a repository offers.
///
/// Projectors are left out: an `mmproj` is a companion to a vision model, not a
/// model to load on its own, and listing it beside the weights would offer a file
/// that cannot be run. It still installs -- a later download of the same
/// repository writes into the same folder.
pub fn gguf_files(repo: &str, tree: &[TreeEntry]) -> Vec<DownloadFile> {
    tree.iter()
        .filter(|entry| entry.entry_type == "file")
        .filter(|entry| entry.path.to_ascii_lowercase().ends_with(".gguf"))
        .filter(|entry| !entry.path.to_ascii_lowercase().contains("mmproj"))
        .map(|entry| DownloadFile {
            repo: repo.to_string(),
            path: entry.path.clone(),
            name: file_name(&entry.path).to_string(),
            size: entry.size,
            note: quantisation(&entry.path).unwrap_or_default(),
            group: None,
        })
        .collect()
}

/// Every voice in Piper's index, filtered by `query` and ordered English first.
///
/// The index is one file rather than a search: the voices all live in a single
/// repository, so there is no list of repositories to ask about, and the index
/// already carries every size. One request answers the whole page.
pub fn piper_voices(index: &serde_json::Value, query: &str) -> Vec<DownloadFile> {
    let needle = query.trim().to_ascii_lowercase();
    let Some(voices) = index.as_object() else {
        return Vec::new();
    };

    let mut rows: Vec<(bool, DownloadFile)> = Vec::new();
    for (key, voice) in voices {
        let Some(model) = voice
            .get("files")
            .and_then(|files| files.as_object())
            .and_then(|files| {
                files
                    .iter()
                    .find(|(path, _)| path.to_ascii_lowercase().ends_with(".onnx"))
            })
        else {
            continue;
        };

        let language = voice
            .get("language")
            .and_then(|value| value.get("name_english"))
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let family = voice
            .get("language")
            .and_then(|value| value.get("family"))
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let quality = voice
            .get("quality")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let name = voice
            .get("name")
            .and_then(|value| value.as_str())
            .unwrap_or(key);

        // The search reads the name a reader knows -- the voice, its language,
        // its quality -- rather than the file path, which nobody types.
        if !needle.is_empty() {
            let haystack = format!("{key} {name} {language} {quality}").to_ascii_lowercase();
            if !haystack.contains(&needle) {
                continue;
            }
        }

        let size = model
            .1
            .get("size_bytes")
            .and_then(|value| value.as_u64())
            .unwrap_or(0);

        rows.push((
            family == "en",
            DownloadFile {
                repo: PIPER_REPO.to_string(),
                path: model.0.clone(),
                name: file_name(model.0).to_string(),
                size,
                note: format!("{} · {}", language, quality),
                group: None,
            },
        ));
    }

    // English first because this app's own voice is English and a list sorted by
    // path would open on Arabic; then by name, so the order is stable between
    // two searches that return the same set.
    rows.sort_by(|(left_english, left), (right_english, right)| {
        right_english
            .cmp(left_english)
            .then_with(|| left.name.cmp(&right.name))
    });
    rows.into_iter().map(|(_, file)| file).collect()
}

/// The one repository the Piper voices live in.
pub const PIPER_REPO: &str = "rhasspy/piper-voices";

/// The recognition models' repositories.
///
/// Two repositories because they are two model families with two file formats,
/// both read by the whisper.cpp engine: Whisper's own ggml models, and the
/// Parakeet TDT port shipped inside whisper.cpp. A `parakeet.cpp` GGUF is a
/// different format and would not load, which is why the Parakeet source is
/// named here rather than left to a search.
pub const WHISPER_REPO: &str = "ggerganov/whisper.cpp";
pub const PARAKEET_REPO: &str = "ggml-org/parakeet-GGUF";

/// Where audio.cpp's ready-to-run packages live: one repository, one tree.
///
/// The family a package belongs to is carried in its file name -- `Kokoro-...`,
/// `qwen3-asr-...` -- so a search that reads one tree answers the whole page,
/// the way the Piper voices do.
pub const AUDIO_REPO: &str = "audio-cpp/audio.cpp-gguf";

/// The community wake-word collection: openWakeWord classifier heads, one file
/// per word. Only `en/` is listed, since the rows carry their own path and the
/// other languages are a different phrase set entirely.
pub const WAKEWORDS_REPO: &str = "fwartner/home-assistant-wakewords-collection";

pub fn collection_heads(repo: &str, tree: &[TreeEntry], query: &str) -> Vec<DownloadFile> {
    let needle = query.trim().to_ascii_lowercase();
    tree.iter()
        .filter(|entry| entry.entry_type == "file")
        .filter(|entry| entry.path.starts_with("en/"))
        .filter(|entry| entry.path.to_ascii_lowercase().ends_with(".onnx"))
        .filter(|entry| needle.is_empty() || entry.path.to_ascii_lowercase().contains(&needle))
        .map(|entry| DownloadFile {
            repo: repo.to_string(),
            path: entry.path.clone(),
            name: file_name(&entry.path).to_string(),
            size: entry.size,
            note: "openWakeWord · English".to_string(),
            // These run in-process through ONNX Runtime, while a GGUF wake pack
            // runs in the audio.cpp server: the runtime is what a reader is
            // choosing between, so that is the section.
            group: Some("openWakeWord".to_string()),
        })
        .collect()
}

/// The `.gguf` packages an audio.cpp repository offers, labelled by family.
pub fn audio_packages(repo: &str, tree: &[TreeEntry], query: &str) -> Vec<DownloadFile> {
    use crate::audio::audiocpp::families::{self, Task};

    let needle = query.trim().to_ascii_lowercase();
    tree.iter()
        .filter(|entry| entry.entry_type == "file")
        .filter(|entry| entry.path.to_ascii_lowercase().ends_with(".gguf"))
        .filter(|entry| needle.is_empty() || entry.path.to_ascii_lowercase().contains(&needle))
        .filter_map(|entry| {
            let name = file_name(&entry.path);
            // The family is what makes a row usable or not: a package the table
            // does not name is installable but unusable, and saying so is more
            // honest than a bare quantisation note.
            let family = families::classify(name);
            let (note, group) = match &family {
                Some((family, task)) => {
                    // The task is the section: a reader looking for a voice is
                    // not looking through recognition models for it.
                    let section = match task {
                        Task::Tts => "speech",
                        Task::Asr => "recognition",
                        Task::Wake => "wake word",
                    };
                    (
                        format!("{} · {}", family.replace('_', " "), section),
                        Some(section.to_string()),
                    )
                }
                None => (
                    "unknown family".to_string(),
                    Some("unknown family".to_string()),
                ),
            };
            Some(DownloadFile {
                repo: repo.to_string(),
                path: entry.path.clone(),
                name: name.to_string(),
                size: entry.size,
                note,
                group,
            })
        })
        .collect()
}

/// The same audio.cpp tree, narrowed to the wake-word families.
///
/// One catalogue, two pages: the models page shows everything the engine can
/// run, and the wake page shows what it can listen for. Filtering here rather
/// than in the page keeps the rule next to the classification it depends on.
pub fn wake_packages(repo: &str, tree: &[TreeEntry], query: &str) -> Vec<DownloadFile> {
    use crate::audio::audiocpp::families::{self, Task};

    audio_packages(repo, tree, query)
        .into_iter()
        .filter(|file| matches!(families::classify(&file.name), Some((_, Task::Wake))))
        // On this page the question is which runtime runs the pack, not what it
        // does: both sources detect a word, and only one of them needs the
        // audio.cpp server. The section is replaced rather than kept.
        .map(|mut file| {
            file.group = Some("audio.cpp".to_string());
            file
        })
        .collect()
}

/// The `.bin` models a recognition repository offers.
pub fn stt_models(repo: &str, tree: &[TreeEntry], query: &str) -> Vec<DownloadFile> {
    let needle = query.trim().to_ascii_lowercase();
    tree.iter()
        .filter(|entry| entry.entry_type == "file")
        .filter(|entry| entry.path.to_ascii_lowercase().ends_with(".bin"))
        // Root files only: whisper.cpp keeps test fixtures and converted copies in
        // subfolders, and none of those is a model to install.
        .filter(|entry| !entry.path.contains('/'))
        .filter(|entry| needle.is_empty() || entry.path.to_ascii_lowercase().contains(&needle))
        .map(|entry| DownloadFile {
            repo: repo.to_string(),
            path: entry.path.clone(),
            name: file_name(&entry.path).to_string(),
            size: entry.size,
            note: recognition_note(repo, &entry.path),
            group: None,
        })
        .collect()
}

/// What tells two recognition models apart: language, and a quantisation if the
/// file name states one.
fn recognition_note(repo: &str, path: &str) -> String {
    if repo == PARAKEET_REPO {
        return "Parakeet TDT · multilingual".to_string();
    }
    let name = file_name(path).to_ascii_lowercase();
    let mut note = if name.contains(".en") {
        "English only".to_string()
    } else {
        "multilingual".to_string()
    };
    if let Some(at) = name.find("-q") {
        note.push_str(" · ");
        note.push_str(&name[at + 1..].trim_end_matches(".bin").to_ascii_uppercase());
    }
    note
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, size: u64) -> TreeEntry {
        TreeEntry {
            entry_type: "file".to_string(),
            path: path.to_string(),
            size,
        }
    }

    #[test]
    fn a_projector_is_not_offered_as_a_model() {
        let tree = vec![
            file("tiny-Q4_K_M.gguf", 10),
            file("mmproj-tiny-f16.gguf", 5),
        ];
        let files = gguf_files("acme/tiny", &tree);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "tiny-Q4_K_M.gguf");
        assert_eq!(files[0].note, "Q4_K_M");
    }

    #[test]
    fn a_directory_is_not_a_file() {
        let mut directory = file("nested", 0);
        directory.entry_type = "directory".to_string();
        assert!(gguf_files("acme/tiny", &[directory]).is_empty());
    }

    #[test]
    fn the_longest_quantisation_name_wins() {
        // `Q2_K` is a prefix of `Q2_K_S`, and only the file name says which one
        // this is.
        assert_eq!(quantisation("m-Q2_K_S.gguf").as_deref(), Some("Q2_K_S"));
        assert_eq!(quantisation("m-Q4_K_M.gguf").as_deref(), Some("Q4_K_M"));
        assert_eq!(quantisation("m-q8_0.gguf"), None);
        assert_eq!(quantisation("m-f16.gguf").as_deref(), Some("F16"));
    }

    fn index() -> serde_json::Value {
        serde_json::json!({
            "ar_JO-kareem-low": {
                "name": "kareem",
                "quality": "low",
                "language": { "family": "ar", "name_english": "Arabic" },
                "files": {
                    "ar/ar_JO/kareem/low/ar_JO-kareem-low.onnx": { "size_bytes": 10 },
                    "ar/ar_JO/kareem/low/ar_JO-kareem-low.onnx.json": { "size_bytes": 1 }
                }
            },
            "en_US-amy-low": {
                "name": "amy",
                "quality": "low",
                "language": { "family": "en", "name_english": "English" },
                "files": {
                    "en/en_US/amy/low/en_US-amy-low.onnx": { "size_bytes": 20 },
                    "en/en_US/amy/low/en_US-amy-low.onnx.json": { "size_bytes": 1 }
                }
            }
        })
    }

    #[test]
    fn the_model_is_chosen_rather_than_its_settings_file() {
        let files = piper_voices(&index(), "");
        assert_eq!(files.len(), 2);
        assert!(files.iter().all(|file| file.name.ends_with(".onnx")));
        assert_eq!(
            files.iter().map(|file| file.size).sum::<u64>(),
            30,
            "the .onnx.json must not become a row"
        );
    }

    #[test]
    fn english_voices_are_listed_first() {
        let files = piper_voices(&index(), "");
        assert_eq!(files[0].name, "en_US-amy-low.onnx");
        assert_eq!(files[0].note, "English · low");
    }

    #[test]
    fn a_search_reads_the_language_and_the_voice() {
        assert_eq!(piper_voices(&index(), "arabic").len(), 1);
        assert_eq!(piper_voices(&index(), "amy").len(), 1);
        assert!(piper_voices(&index(), "klingon").is_empty());
    }

    #[test]
    fn an_audio_package_carries_the_section_it_belongs_to() {
        // The page's section filter reads this, so a row that names its task is
        // what keeps a voice search from returning recognition models.
        let tree = vec![
            file("Kokoro-82M-Q8_0.gguf", 10),
            file("qwen3-asr-0.6b-q8_0.gguf", 20),
            file("open-wake-word-alexa-f32.gguf", 30),
        ];
        let files = audio_packages(AUDIO_REPO, &tree, "");
        assert_eq!(files[0].group.as_deref(), Some("speech"));
        assert_eq!(files[1].group.as_deref(), Some("recognition"));
        assert_eq!(files[2].group.as_deref(), Some("wake word"));
    }

    #[test]
    fn a_wake_package_is_grouped_by_the_runtime_that_runs_it() {
        // On the wake page every row detects a word, so the useful section is
        // which runtime needs to be installed for it.
        let tree = vec![
            file("Kokoro-82M-Q8_0.gguf", 10),
            file("open-wake-word-alexa-f32.gguf", 30),
        ];
        let files = wake_packages(AUDIO_REPO, &tree, "");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].group.as_deref(), Some("audio.cpp"));
    }

    #[test]
    fn a_single_list_kind_offers_no_section() {
        // The page hides its filter when there is nothing to filter by, which is
        // what an absent group says.
        let files = gguf_files("acme/tiny", &[file("tiny-Q4_K_M.gguf", 10)]);
        assert_eq!(files[0].group, None);
    }
}
