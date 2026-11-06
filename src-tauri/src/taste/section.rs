//! The fixed set of headings a learned fact may live under.
//!
//! A closed set is what keeps the settings page's grouping comparable between
//! runs: a model that could invent headings would eventually produce one section
//! per conversation. The model never names one -- [`Section::classify`] picks it
//! from the fact's own words, which costs no output tokens and cannot fail.

use serde::{Deserialize, Serialize};

/// One heading in the profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Section {
    /// Durable facts about the person and their machine.
    Identity,
    /// What they build, the stack they use, and how they work.
    Work,
    /// Explicit likes and choices.
    Preferences,
    /// How they want to be spoken to and written for.
    Communication,
    /// Things they follow, make or do for their own sake.
    Interests,
    /// Explicit dislikes and things that annoy them.
    Dislikes,
    /// A focus that is true now and likely to expire.
    Current,
}

impl Section {
    /// The stored name.
    pub fn as_str(self) -> &'static str {
        match self {
            Section::Identity => "identity",
            Section::Work => "work",
            Section::Preferences => "preferences",
            Section::Communication => "communication",
            Section::Interests => "interests",
            Section::Dislikes => "dislikes",
            Section::Current => "current",
        }
    }

    /// The section a fact belongs under.
    ///
    /// **Chosen here, not by the model.** A section is a display grouping, and
    /// asking a small model to name one costs tokens on every line and gives it
    /// one more way to produce an unusable answer. The rules are ordered: a fact
    /// that names both a dislike and a preference is a dislike, and one that says
    /// "currently" is a current focus rather than work.
    ///
    /// A fact that matches nothing is a preference, which is the broadest heading
    /// and the one a fact about a person most often belongs under.
    pub fn classify(text: &str) -> Self {
        let words = words(text);
        let has = |needles: &[&str]| {
            needles.iter().any(|needle| {
                let needle = fold_word(needle);
                words.iter().any(|word| word == &needle)
            })
        };
        let phrase =
            |needles: &[&str]| needles.iter().any(|needle| contains_phrase(&words, needle));

        if phrase(&[
            "currently",
            "right now",
            "these days",
            "at the moment",
            "this week",
            "working on",
        ]) {
            return Section::Current;
        }
        if has(&[
            "dislike", "dislikes", "hate", "hates", "avoid", "avoids", "annoying", "annoys",
        ]) || phrase(&[
            "cant stand",
            "cannot stand",
            "dont like",
            "does not like",
            "not a fan",
        ]) {
            return Section::Dislikes;
        }
        if has(&[
            "concise",
            "brief",
            "verbosity",
            "tone",
            "markdown",
            "emojis",
            "wording",
        ]) || phrase(&["bullet points", "short answers"])
        {
            return Section::Communication;
        }
        // Work before identity: "Builds Tauri apps for macOS and Windows" is a
        // work fact that happens to name two platforms, and the platform words
        // must not outrank the verb.
        if has(&[
            "work",
            "works",
            "job",
            "developer",
            "programmer",
            "engineer",
            "builds",
            "code",
            "coding",
            "programming",
            "stack",
            "backend",
            "frontend",
            "api",
            "database",
            "project",
            "rust",
            "python",
            "typescript",
            "javascript",
            "react",
            "tauri",
            "node",
            "deno",
            "java",
            "swift",
            "kotlin",
            "php",
            "ruby",
            "html",
            "css",
            "sql",
            "docker",
            "kubernetes",
            "git",
        ]) {
            return Section::Work;
        }
        if has(&[
            "name", "lives", "live", "age", "windows", "linux", "macos", "gpu", "cpu", "rtx",
            "radeon", "laptop", "desktop", "monitor", "keyboard", "mouse", "glasses", "macbook",
            "thinkpad", "ram", "iphone", "android",
        ]) || phrase(&["years old"])
        {
            return Section::Identity;
        }
        if has(&[
            "games",
            "gaming",
            "music",
            "guitar",
            "piano",
            "draw",
            "draws",
            "drawing",
            "painting",
            "reads",
            "reading",
            "books",
            "movies",
            "films",
            "anime",
            "manga",
            "art",
            "hiking",
            "cooking",
            "baking",
            "chess",
            "photography",
            "travel",
        ]) {
            return Section::Interests;
        }
        Section::Preferences
    }

    /// Parses a stored or model-supplied name.
    ///
    /// Lowercased and trimmed because a model writes `Work` as readily as `work`,
    /// and a section that differs only in case is not a different section.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "identity" => Some(Section::Identity),
            "work" => Some(Section::Work),
            "preferences" => Some(Section::Preferences),
            "communication" => Some(Section::Communication),
            "interests" => Some(Section::Interests),
            "dislikes" => Some(Section::Dislikes),
            "current" => Some(Section::Current),
            _ => None,
        }
    }
}

/// The words of a fact, lowercased, with punctuation and apostrophes folded away.
///
/// Apostrophes go rather than splitting a word: `don't` and `dont` are the same
/// word for every rule here, and keeping the mark would make every contraction
/// need two spellings in the lists.
fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .replace(['\'', '’'], "")
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(fold_word)
        .collect()
}

/// The same plural fold the similarity check uses, so "monitors" matches
/// "monitor" instead of needing two spellings in every list. Applied to the
/// needles as well as the text, which is why the lists may be written naturally.
fn fold_word(word: &str) -> String {
    if word.len() > 4 && word.ends_with('s') {
        word[..word.len() - 1].to_string()
    } else {
        word.to_string()
    }
}

/// Whether a phrase appears as consecutive words.
fn contains_phrase(words: &[String], phrase: &str) -> bool {
    let needle: Vec<String> = phrase.split_whitespace().map(fold_word).collect();
    !needle.is_empty() && words.windows(needle.len()).any(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_section_is_chosen_from_the_fact_itself() {
        assert_eq!(
            Section::classify("Uses Rust (programming language)."),
            Section::Work
        );
        assert_eq!(
            Section::classify("Uses Windows 11 on an RTX 5090 (GPU)."),
            Section::Identity
        );
        assert_eq!(
            Section::classify("Dislikes loud notifications."),
            Section::Dislikes
        );
        assert_eq!(
            Section::classify("Prefers concise answers with no emojis."),
            Section::Communication
        );
        assert_eq!(
            Section::classify("Currently learning Japanese."),
            Section::Current
        );
        assert_eq!(
            Section::classify("Plays chess on weekends."),
            Section::Interests
        );
        assert_eq!(
            Section::classify("Prefers dark themes."),
            Section::Preferences
        );
        // Work outranks a platform named in the same sentence, and a plural is
        // folded so "monitors" matches the rule written for "monitor".
        assert_eq!(
            Section::classify("Builds Tauri apps for macOS and Windows."),
            Section::Work
        );
        assert_eq!(Section::classify("Uses 4K monitors."), Section::Identity);
    }

    #[test]
    fn names_round_trip_and_ignore_case() {
        let sections = [
            Section::Identity,
            Section::Work,
            Section::Preferences,
            Section::Communication,
            Section::Interests,
            Section::Dislikes,
            Section::Current,
        ];
        for section in sections {
            assert_eq!(Section::parse(section.as_str()), Some(section));
            assert_eq!(
                Section::parse(&section.as_str().to_ascii_uppercase()),
                Some(section)
            );
        }
    }

    #[test]
    fn an_unknown_section_is_refused_rather_than_guessed_at() {
        assert_eq!(Section::parse("misc"), None);
        assert_eq!(Section::parse(""), None);
    }
}
