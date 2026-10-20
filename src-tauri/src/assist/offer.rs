//! What kind of help a block deserves.
//!
//! # Why this is not [`super::candidate::Shape`]
//!
//! `Shape` is *evidence*: the cheap, string-level question "does this look like a sum, a
//! riddle, a question or just prose?" It is decided before a model is contacted, and it is
//! deliberately conservative.
//!
//! `Kind` is *the offer*: what the app would do about the block -- answer it, explain it,
//! summarise it, translate it, or check it. It is decided by the model (or by the shape,
//! when the model is not consulted), and it is what the card is drawn as.
//!
//! Keeping them apart is what lets the feature stop being "a question answerer" without
//! making the gate any less careful. A block can be `Shape::Passage` and `Kind::Summarize`;
//! the gate never had to know that summarising was possible.
//!
//! # Why the set is closed
//!
//! A 2B asked to invent a category invents a different one every time. Five labels with a
//! sentence each are something a small model can hold, and -- more importantly -- something
//! the parser can map back to one of five renderings without guessing. A label outside the
//! set is treated as `Answer`, which is the conservative fallback: the content is still
//! usable, and no new surface is created.

use serde::{Deserialize, Serialize};

use super::candidate::Shape;

/// What the app will do about a block, if anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// The block asks something, and the answer is the content.
    Answer,
    /// The block states an idea, term or passage worth a short explanation.
    Explain,
    /// The block is a section long enough to be worth condensing.
    Summarize,
    /// The block is in a language other than the page's, and the content is a translation.
    Translate,
    /// The block is code, a calculation or a claim that can be checked.
    Check,
}

impl Kind {
    /// Every kind, in the order a prompt lists them.
    pub const ALL: [Kind; 5] = [
        Kind::Answer,
        Kind::Explain,
        Kind::Summarize,
        Kind::Translate,
        Kind::Check,
    ];

    /// The wire name, the same one serde writes.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Answer => "answer",
            Kind::Explain => "explain",
            Kind::Summarize => "summarize",
            Kind::Translate => "translate",
            Kind::Check => "check",
        }
    }

    /// Reads a label a model wrote, or `None` for anything that is not one of the five.
    ///
    /// Aliases are here because a model told "summarize" may write "summary", and refusing
    /// that would throw away a correct judgement over a suffix.
    pub fn parse(raw: &str) -> Option<Self> {
        let word = raw
            .trim()
            .trim_matches(|character: char| !character.is_alphanumeric())
            .to_lowercase();
        match word.as_str() {
            "answer" | "answers" => Some(Kind::Answer),
            "explain" | "explanation" | "explain_this" => Some(Kind::Explain),
            "summarize" | "summarise" | "summary" => Some(Kind::Summarize),
            "translate" | "translation" => Some(Kind::Translate),
            "check" | "verify" | "debug" | "review" => Some(Kind::Check),
            _ => None,
        }
    }

    /// The kind a block gets when the model is not asked, from the gate's own evidence.
    ///
    /// A passage is prose, so explaining it is the only useful thing to do; everything the
    /// gate could name is a question in some form, so answering is.
    pub fn from_shape(shape: Shape) -> Self {
        match shape {
            Shape::Passage => Kind::Explain,
            Shape::Arithmetic | Shape::Question | Shape::Riddle => Kind::Answer,
        }
    }

    /// How much a card of this kind may say, in words.
    ///
    /// A short answer is a glance; a summary is a paragraph. The number is a property of
    /// the card rather than of the request, so it is the same at every level -- a level
    /// changes *which* kinds are offered, not how much a kind says.
    pub fn max_words(self) -> usize {
        match self {
            Kind::Answer => 60,
            Kind::Explain => 120,
            Kind::Summarize => 160,
            Kind::Translate => 160,
            Kind::Check => 120,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every kind round-trips through its wire name, which is what the card and the
    /// extension match on.
    #[test]
    fn every_kind_round_trips_through_its_name() {
        for kind in Kind::ALL {
            assert_eq!(Kind::parse(kind.as_str()), Some(kind));
        }
    }

    /// The aliases a model actually writes are read rather than discarded.
    #[test]
    fn aliases_are_understood() {
        assert_eq!(Kind::parse("Summary"), Some(Kind::Summarize));
        assert_eq!(Kind::parse(" explanation "), Some(Kind::Explain));
        assert_eq!(Kind::parse("verify"), Some(Kind::Check));
        assert_eq!(Kind::parse("none"), None);
        assert_eq!(Kind::parse(""), None);
        assert_eq!(Kind::parse("something else"), None);
    }

    /// The unasked default follows the gate: a passage is explained, a question answered.
    #[test]
    fn a_kind_follows_the_shape_when_the_model_does_not_choose() {
        assert_eq!(Kind::from_shape(Shape::Arithmetic), Kind::Answer);
        assert_eq!(Kind::from_shape(Shape::Question), Kind::Answer);
        assert_eq!(Kind::from_shape(Shape::Riddle), Kind::Answer);
        assert_eq!(Kind::from_shape(Shape::Passage), Kind::Explain);
    }
}
