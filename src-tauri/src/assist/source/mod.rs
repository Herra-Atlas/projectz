//! Where the text comes from.
//!
//! # One source, and one seam
//!
//! Everything after this module is the same whatever the text came from: the same gate,
//! the same request, the same verdict. A source's whole job is to say what is on screen.
//!
//! There is one source. The browser is it, reached through the extension the app serves
//! and pairs with -- because a browser the user opened themselves has no debugging port,
//! one cannot be opened on a process already running, and there is no way in from the
//! outside. See [`browser`] for the whole of that reasoning.
//!
//! [`apps`] is the seam for the other one: reading an ordinary Windows application, which
//! means an accessibility tree where one exists and a screenshot and a vision model where
//! it does not. It is documentation and nothing else, deliberately. A source that is not
//! built is not listed in the settings page either: a row saying "not yet" explains
//! nothing a user can act on.

pub mod apps;
pub mod browser;

use serde::{Deserialize, Serialize};

/// A rectangle on screen, in CSS pixels once it reaches the frontend.
///
/// Carried from the source rather than measured again in the app: the source is
/// the only thing that knows where a block is, and a second measurement in another
/// process would be a second answer about the same page.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// One block of text, as the source sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
    /// Where this block sits in the frame, 1-based.
    ///
    /// **The number is the whole contract between the model and the page.** The
    /// model is shown the blocks numbered and answers by number, and the source
    /// draws each card beside the block that number names -- so a page with five
    /// questions gets five cards in five places rather than one card with the
    /// wrong answer in it. Assigned by the source, never renumbered here: the
    /// source is the only thing that knows where the block is.
    ///
    /// A *number*, though, and not a name: it says where a block was in the frame it was sent with.
    /// If the page changes while the request is in flight, that number can come to mean a different
    /// paragraph -- which is what [`Self::key`] is for.
    pub index: usize,
    /// The block's identity, as the source knows it: stable across a page that changes under it.
    ///
    /// Indices shift when the page does -- a block appears above the answer, and every number after
    /// it moves -- so an answer arriving after a change can name a number that now belongs to
    /// something else. **The key is what a card is placed by**: an answer carrying a key the source
    /// can no longer find is dropped, rather than drawn over whatever that number has become. Empty
    /// from a source that does not name its blocks, and then the number is all there is.
    #[serde(default)]
    pub key: String,
    pub text: String,
    /// What the source calls this block -- an element name, a role, a window.
    ///
    /// Not used for the decision, only for the debug view: when a screen's text
    /// is being misread, "which part of it" is the first question.
    #[serde(default)]
    pub tag: String,
    /// What the source already knows about the block's *shape in the page*.
    ///
    /// The text alone says `print(x)` and `Preheat the oven`, and a small model has to
    /// guess which is code. The source has the DOM and knows for free -- `code`, `table`,
    /// `image`, `heading` -- so the hint travels beside the text and the prompt can say
    /// `[3] (code) ...`. Absent is normal and means "nothing structural to say".
    #[serde(default)]
    pub hint: String,
    /// Where it is, when the source knows. Absent is normal: the app's own tests
    /// and its paste-a-block box have nothing to point at.
    #[serde(default)]
    pub rect: Option<Rect>,
}

/// One look at a source: the blocks it can see, in the order it sees them.
///
/// A frame rather than a single block because a page is what a person is looking
/// at, and the question is usually not the only text on it. Sending the frame lets
/// one request answer the whole thing -- several questions on one quiz page cost
/// one call, not one per question -- and it is what makes a numbered answer
/// possible at all.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    /// The source's id for this look, echoed back on the answer so a card can be
    /// matched to the page state it was drawn for.
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub title: String,
    pub blocks: Vec<Block>,
}

impl Frame {
    /// A frame of one block, for the places with nothing to point at.
    ///
    /// The tests use this, and only them: the settings page's try-it box builds its frame
    /// in TypeScript, where the same shape is one object literal. The pipeline is the same
    /// one a page takes, and a single block is a page with one block on it.
    #[cfg(test)]
    pub fn single(text: impl Into<String>) -> Self {
        Self {
            id: String::new(),
            url: String::new(),
            title: String::new(),
            blocks: vec![Block {
                index: 1,
                key: String::new(),
                text: text.into(),
                tag: "preview".to_string(),
                hint: String::new(),
                rect: None,
            }],
        }
    }
}
