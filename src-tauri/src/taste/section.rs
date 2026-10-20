//! The fixed set of headings a learned fact may live under.
//!
//! A closed set is what keeps the profile compact and comparable between runs:
//! a model that could invent headings would eventually produce one section per
//! conversation, and the "same fact, different wording" check would have no
//! container to work in. The prompt names this list, and the merge drops an
//! operation that uses anything else rather than guessing which one was meant.

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
    /// Every section, in the order the profile is rendered.
    ///
    /// Identity first because it is the context the rest is read against, and
    /// `current` last because it is the part expected to go stale. The order is
    /// fixed rather than derived, so a reorder is a deliberate edit to this list
    /// and not a side effect of a map's iteration order.
    pub const ALL: [Section; 7] = [
        Section::Identity,
        Section::Work,
        Section::Preferences,
        Section::Communication,
        Section::Interests,
        Section::Dislikes,
        Section::Current,
    ];

    /// The stored and prompt-facing name.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_ignore_case() {
        for section in Section::ALL {
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
