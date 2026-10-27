//! The audio.cpp model families this app knows, and what task each is for.
//!
//! audio.cpp carries a hundred-plus model families, and its server needs each
//! model's family id when a model is used. The table below is data rather than
//! code on purpose: a new family this app should support is one row here, not a
//! search through the engine's own internals.
//!
//! # How a file name becomes a family
//!
//! Package names are inconsistent -- some carry the family's full id
//! (`qwen3-asr-0.6b`), others only its stem (`Kokoro-82M`), and many drop the
//! `_tts` the family id has. So a name is matched by the closest identifying
//! word, and the words that name a task outright (`asr`, `tts`, ...) break the
//! tie for the stems that back both a TTS and an ASR family. A file that names
//! no family at all stays *unknown*, and an unknown model is refused by name
//! rather than attempted: the server would load it as the wrong architecture
//! otherwise, which is a failure much further from the user than this one.

use std::path::{Path, PathBuf};

/// What a model does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Task {
    /// Text to speech.
    Tts,
    /// Speech to text.
    Asr,
    /// Wake-word detection: is the phrase this model was made for being said.
    Wake,
}

/// One row of the family table.
///
/// `token` is the shortest word that identifies the family in a package name.
/// `tts`, `asr` and `wake` are the family ids to send for each task; a row with
/// more than one is ambiguous on its own, and the file name has to say which
/// side it is.
struct Family {
    token: &'static str,
    tts: Option<&'static str>,
    asr: Option<&'static str>,
    wake: Option<&'static str>,
}

const fn family(token: &'static str, tts: Option<&'static str>) -> Family {
    Family {
        token,
        tts,
        asr: None,
        wake: None,
    }
}

const fn recognition(token: &'static str, asr: &'static str) -> Family {
    Family {
        token,
        tts: None,
        asr: Some(asr),
        wake: None,
    }
}

const fn twin(token: &'static str, tts: &'static str, asr: &'static str) -> Family {
    Family {
        token,
        tts: Some(tts),
        asr: Some(asr),
        wake: None,
    }
}

const fn wake_word(token: &'static str, id: &'static str) -> Family {
    Family {
        token,
        tts: None,
        asr: None,
        wake: Some(id),
    }
}

/// Longest token first, so `drama3` is matched before the `fish` that also
/// appears in the same name.
const FAMILIES: &[Family] = &[
    // The three wake-word families the engine ships specs for. `micro` is a row
    // of its own rather than left to the spec folder: a machine with no engine
    // installed still has to name the file it holds, and the table is what
    // answers that.
    wake_word("micro", "micro_wake_word"),
    wake_word("wake", "open_wake_word"),
    wake_word("kws", "sherpa_kws"),
    family("drama3", Some("fish_drama3")),
    twin("qwen3", "qwen3_tts", "qwen3_asr"),
    twin("index", "index_tts2", "index_echo"),
    twin("moss", "moss_tts_local", "moss_transcribe"),
    twin("higgs", "higgs_audio_tts", "higgs_audio_stt"),
    twin("vibevoice", "vibevoice", "vibevoice_asr"),
    twin("firered", "fireredtts3", "firered_audio"),
    twin("audio8", "audio8_tts", "audio8_asr"),
    twin("confucius", "confucius4_tts", "confucius4_r2t2"),
    // Chatterbox ships as two families the token alone cannot split; the
    // longer row names the Turbo variant, and the shorter one stays the base.
    family("chatterbox_turbo", Some("chatterbox_turbo")),
    twin("chatterbox", "chatterbox", "chatterbox"),
    family("piper", Some("piper_tts")),
    family("kitten", Some("kitten_tts2")),
    family("soprano", Some("soprano_tts")),
    family("sopro", Some("sopro_tts")),
    family("inflect", Some("inflect_v2")),
    family("kokoro", Some("kokoro_tts")),
    family("supertonic", Some("supertonic")),
    family("pocket", Some("pocket_tts")),
    family("fish", Some("fish_audio")),
    family("dots", Some("dots_tts")),
    family("magpie", Some("magpie_tts")),
    family("maya", Some("maya1")),
    family("cosy", Some("cosyvoice3")),
    family("breeze", Some("breeze_tts")),
    family("omnivoice", Some("omnivoice")),
    family("glm", Some("glm_tts")),
    family("voxcpm", Some("voxcpm2")),
    family("kugel", Some("kugelaudio")),
    family("irodori", Some("irodori_tts")),
    family("plapre", Some("plapre")),
    family("echo", Some("echo_tts")),
    family("sano", Some("sanotts")),
    family("ming", Some("ming3_tts")),
    family("chatterbox", Some("chatterbox")),
    family("neutts", Some("neutts")),
    family("vevo", Some("vevo2")),
    family("oni", Some("omnivoice")),
    recognition("whisper", "whistle_asr"),
    recognition("whistle", "whistle_asr"),
    recognition("parakeet", "parakeet_tdt"),
    recognition("moonshine", "moonshine_asr"),
    recognition("sense", "sense_asr"),
    recognition("crisper", "crisperwhisper"),
    recognition("citrinet", "citrinet_asr"),
    recognition("canary", "canary_asr"),
    recognition("gigaam", "gigaam_asr"),
    recognition("kroko", "kroko_asr"),
    recognition("hviske", "hviske_asr"),
    recognition("granite", "granite5asr"),
    recognition("niagara", "niagara_asr"),
    recognition("owsm", "owsm"),
    recognition("vibeasr", "vibeasr"),
];

/// Words that name a task outright, whatever family they ride with.
fn task_words(file_words: &[String]) -> Option<Task> {
    let asr = [
        "asr",
        "stt",
        "transcribe",
        "transcription",
        "echo",
        "whisper",
    ];
    for word in file_words {
        if asr.iter().any(|token| word.starts_with(token)) {
            return Some(Task::Asr);
        }
    }
    for word in file_words {
        if ["tts", "speech", "voice"]
            .iter()
            .any(|token| word.starts_with(token))
        {
            return Some(Task::Tts);
        }
    }
    None
}

/// The family and task a file name is about, or `None` for an unknown one.
pub fn classify(name: &str) -> Option<(String, Task)> {
    let file_words: Vec<String> = name
        .to_ascii_lowercase()
        .split(['.', '_', '-', ' ', '(', ')'])
        .filter(|word| !word.is_empty())
        .map(|word| word.to_string())
        .collect();

    // The longest identifying token wins, whatever word it appears in -- the
    // specificity test, not the position test: `Fish-Speech-Drama3-small`
    // names two families' stems, and `drama3` is the one that means it.
    // Earliest word breaks a length tie, so the order the words appear in is
    // still the order a human reads.
    let mut found: Option<(usize, usize, &Family)> = None;
    for entry in FAMILIES {
        for (index, word) in file_words.iter().enumerate() {
            if !word.starts_with(entry.token) {
                continue;
            }
            let better = match found {
                None => true,
                Some((word_at, token_len, _)) => {
                    entry.token.len() > token_len
                        || (entry.token.len() == token_len && index < word_at)
                }
            };
            if better {
                found = Some((index, entry.token.len(), entry));
            }
        }
    }
    let (_, _, entry) = found?;
    let task = match (task_words(&file_words), entry.tts, entry.asr, entry.wake) {
        (Some(task), ..) => task,
        (None, Some(_), _, _) => Task::Tts,
        (None, None, Some(_), _) => Task::Asr,
        (None, None, None, Some(_)) => Task::Wake,
        (None, None, None, None) => return None,
    };
    // The table names first; the engine's own spec folder corrects it. A token
    // match read `chatterbox` and the installed specs say `chatterbox_turbo`
    // for this exact name -- the specs win, because they are what the server
    // verifies the request against.
    let id = match task {
        Task::Tts => entry.tts,
        Task::Asr => entry.asr,
        Task::Wake => entry.wake,
    };
    let id = match id {
        Some(id) => refined_family(&file_words, &id),
        None => return None,
    };
    Some((id, task))
}

/// The engine installs its `model_specs/<family>.json` set, and those stems are
/// the server's own family registry -- every request is verified against the
/// family the GGUF *embedded*, which the specs mirror. When the folder is
/// available, the table's guess is only a candidate: the answer is the spec
/// stem whose parts all appear in the file name, most specific first.
///
/// This is what turned `chatterbox` into `chatterbox_turbo` for
/// `chatterbox-turbo-q8_0.gguf` -- the server rejected the former with
/// "GGUF embeds model spec for family …" and the latter synthesised.
fn refined_family(file_words: &[String], table_id: &str) -> String {
    let mut best: Option<(usize, String)> = None;
    for stem in spec_stems() {
        let parts: Vec<&str> = stem.split('_').collect();
        // Every part of the stem must read in the file name, in order -- the
        // thread is the stem, not the file: `chatterbox_turbo` threading
        // through `chatterbox-turbo-q8_0` qualifies, while `moonshine_asr`
        // threading through `chatterbox-turbo` does not.
        let mut at = 0;
        let qualifies = parts.iter().all(|part| {
            at += file_words[at..]
                .iter()
                .position(|word| word.starts_with(*part))
                .map(|found| found + 1)
                .unwrap_or(usize::MAX - at);
            at <= file_words.len()
        });
        if !qualifies {
            continue;
        }
        let better = match best {
            None => true,
            Some((best_parts, _)) => parts.len() > best_parts,
        };
        if better {
            best = Some((parts.len(), stem));
        }
    }
    best.map(|(_, stem)| stem)
        .unwrap_or_else(|| table_id.to_string())
}

/// The spec stems the installed engine carries, longest first.
fn spec_stems() -> Vec<String> {
    static STEMS: std::sync::LazyLock<Vec<String>> = std::sync::LazyLock::new(|| {
        let Some(directory) =
            crate::audio::engines::audiocpp::installed_dir().map(|dir| dir.join("model_specs"))
        else {
            return Vec::new();
        };
        let Ok(entries) = std::fs::read_dir(&directory) else {
            return Vec::new();
        };
        let mut stems: Vec<String> = entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                (path.extension()?.eq_ignore_ascii_case("json")).then(|| {
                    path.file_stem()
                        .and_then(|stem| stem.to_str())
                        .unwrap_or("")
                        .to_string()
                })
            })
            .filter(|stem| !stem.is_empty())
            .collect();
        stems.sort_by_key(|stem| std::cmp::Reverse(stem.len()));
        stems
    });
    STEMS.clone()
}

/// One installed audio.cpp model: the file, its family, and its task.
#[derive(Clone, Debug)]
pub struct Model {
    /// Absolute path. It is both the id and what the server config names.
    pub path: PathBuf,
    /// The file's own name, which is what a list shows.
    pub name: String,
    /// The family id the server config names, refined against the engine's
    /// shipped specs when it carries them.
    pub family: String,
    pub task: Option<Task>,
    pub size: u64,
}

impl Model {
    /// A short id for the server config, stable for one file on disk.
    pub fn slug(&self) -> String {
        let stem = self
            .path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("model");
        slug(stem)
    }
}

/// Every installed audio.cpp model, walking `models/audio`.
///
/// The folder is the truth, like `models/stt` and `models/tts` are: a file that
/// was deleted has been uninstalled, whatever any record says, and one the user
/// copied in by hand is as installed as one the download page put there.
pub fn installed() -> Vec<Model> {
    let root = crate::app_paths::models_root().join("audio");
    let mut found = Vec::new();
    collect(&root, &mut found);
    found.sort_by(|left, right| left.name.cmp(&right.name));
    found
}

/// Every installed model of one task, a voice list in order.
pub fn installed_for(task: Task) -> Vec<Model> {
    installed()
        .into_iter()
        .filter(|model| model.task == Some(task))
        .collect()
}

fn collect(directory: &Path, found: &mut Vec<Model>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, found);
            continue;
        }
        let is_model = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("gguf"));
        if !is_model {
            continue;
        }
        let name = entry.file_name().to_str().unwrap_or_default().to_string();
        let (family, task) = match classify(&name) {
            Some((family, task)) => (family, Some(task)),
            None => ("unknown".to_string(), None),
        };
        found.push(Model {
            path,
            name,
            family,
            task,
            size: std::fs::metadata(entry.path())
                .map(|meta| meta.len())
                .unwrap_or(0),
        });
    }
}

/// A slug sane for the server config's `id`: lowercase, alphanumerics and
/// dashes, capped so a quantisation-heavy name still fits.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for character in name.chars() {
        if character.is_ascii_alphanumeric() || character == '-' {
            out.push(character.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').chars().take(40).collect::<String>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_voice_package_names_its_family() {
        assert_eq!(
            classify("Fish-Speech-Drama3-small-Q4_K_M.gguf"),
            Some(("fish_drama3".to_string(), Task::Tts))
        );
        assert_eq!(
            classify("Kokoro-82M-Q8_0.gguf"),
            Some(("kokoro_tts".to_string(), Task::Tts))
        );
        assert_eq!(
            classify("piper_tts-lessac-medium.gguf"),
            Some(("piper_tts".to_string(), Task::Tts))
        );
        assert_eq!(
            classify("chatterbox-turbo-q8_0.gguf"),
            Some(("chatterbox_turbo".to_string(), Task::Tts))
        );
        assert_eq!(
            classify("Chatterbox-0.5B-q5_k_m.gguf"),
            Some(("chatterbox".to_string(), Task::Tts))
        );
        assert_eq!(
            classify("inflect-micro-v2-orig.gguf"),
            Some(("inflect_v2".to_string(), Task::Tts))
        );
    }

    #[test]
    fn a_recognition_package_names_its_family() {
        assert_eq!(
            classify("qwen3-asr-0.6b-q8_0.gguf"),
            Some(("qwen3_asr".to_string(), Task::Asr))
        );
        assert_eq!(
            classify("moonshine-small-streaming.gguf"),
            Some(("moonshine_asr".to_string(), Task::Asr))
        );
        assert_eq!(
            classify("parakeet-tdt-0.6b-v3.gguf"),
            Some(("parakeet_tdt".to_string(), Task::Asr))
        );
    }

    #[test]
    fn the_two_sides_of_an_ambiguous_stem_are_told_apart_by_the_name() {
        assert_eq!(
            classify("qwen3-tts-1.7b-base.gguf"),
            Some(("qwen3_tts".to_string(), Task::Tts))
        );
        assert_eq!(
            classify("index-echo-s2tt-9b.gguf"),
            Some(("index_echo".to_string(), Task::Asr))
        );
    }

    #[test]
    fn a_wake_word_package_names_its_family() {
        // The names the download page actually lists, and the names a model
        // trained outside this app has to be given to be recognisable at all.
        assert_eq!(
            classify("open-wake-word-alexa-f32.gguf"),
            Some(("open_wake_word".to_string(), Task::Wake))
        );
        assert_eq!(
            classify("micro-wake-word-okay-nabu-f32.gguf"),
            Some(("micro_wake_word".to_string(), Task::Wake))
        );
        assert_eq!(
            classify("sherpa-kws-zipformer-zh-en-3m-f32.gguf"),
            Some(("sherpa_kws".to_string(), Task::Wake))
        );
        assert_eq!(
            classify("open-wake-word-sara-f32.gguf"),
            Some(("open_wake_word".to_string(), Task::Wake))
        );
    }

    #[test]
    fn an_architecture_the_table_does_not_know_is_unknown() {
        assert_eq!(classify("some-future-model-Q8.gguf"), None);
    }

    #[test]
    fn a_slug_is_lowercase_and_bounded() {
        let slug = slug("Fish-Speech Drama3_small (Q4_K_M).gguf");
        assert!(slug
            .chars()
            .all(|character| { character.is_ascii_alphanumeric() || character == '-' }));
    }
}
