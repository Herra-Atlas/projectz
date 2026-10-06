//! The right panel's backend.
//!
//! App chrome the user drives, kept apart from `ai/tools/` -- see [`fs`] for why
//! the file tree is not a model tool. Each view added to the panel gets a module
//! here; the panel is not one feature, it is a place for several.

pub mod browser;
pub mod fs;
pub mod terminal;
