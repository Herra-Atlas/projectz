//! The JavaScript this driver installs into a page.
//!
//! Two files, kept as JavaScript rather than as a Rust string so they can be read
//! and edited as what they are. Both are injected on every call: the first time
//! it builds the API, and every time after that it costs one no-op because the
//! page already has one. That is also what puts the API back after a navigation,
//! where the page is a new document that has never seen any of this.
//!
//! # Why the injection is idempotent rather than tracked
//!
//! The alternative is remembering, per page, whether the script was installed --
//! and after a navigation that memory is about a document that no longer exists.
//! A guard at the top of each file is one line and cannot be wrong about which
//! document it is looking at.

/// The name of the bridge function, which must match in three places.
///
/// The driver installs it with `Runtime.addBinding`, the prelude wraps it, and
/// the driver answers its calls. A rename in any one of them is a driver that
/// silently stops accepting input, so it is a constant here and a test below.
pub const BINDING: &str = "__pzSend";

/// The DOM tree walker.
const WALKER: &str = include_str!("walker.js");

/// The API a `browser_exec` script is written against.
const PRELUDE: &str = include_str!("prelude.js");

/// What a page is given before a script runs.
///
/// One string rather than two evaluations, because there is no point at which the
/// walker is useful without the API that exposes it, and a call that ran between
/// them would be a call that failed for no reason.
pub fn injection() -> String {
    format!("{WALKER}\n{PRELUDE}\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::bridge;

    /// The three places the bridge name has to agree. A mismatch here is a tool
    /// that looks like it works and never receives a click.
    #[test]
    fn the_bridge_name_is_the_one_the_page_wraps() {
        assert!(
            PRELUDE.contains(BINDING),
            "the prelude must wrap `{BINDING}`, which is what the driver installs"
        );
    }

    /// Every action the prelude sends has an arm on the driver, and vice versa.
    ///
    /// Written as a scan rather than a list because a list is the thing that goes
    /// out of date: this reads the ops out of the JavaScript that sends them and
    /// holds them against the ones the driver answers. It fails both ways -- an
    /// action added to the page with nothing behind it, and an arm left behind
    /// after an action was removed.
    #[test]
    fn every_action_the_page_sends_is_answered_by_the_driver() {
        let page = ops_in(PRELUDE);
        assert!(!page.is_empty(), "no actions were found in the prelude");

        let mut expected = bridge::OPS.to_vec();
        expected.sort_unstable();
        let mut found = page.clone();
        found.sort_unstable();

        assert_eq!(
            found, expected,
            "the prelude and the driver disagree about which actions exist"
        );
    }

    /// Every op the prelude sends, read from the source that sends it.
    ///
    /// Matched on `ask('name'` -- the one shape the prelude uses to reach the
    /// driver -- so a helper that wraps another cannot be mistaken for an op of
    /// its own.
    fn ops_in(source: &str) -> Vec<String> {
        let mut found = Vec::new();
        let mut rest = source;
        while let Some(start) = rest.find("ask('") {
            let after = &rest[start + 5..];
            let Some(end) = after.find('\'') else { break };
            let name = after[..end].to_string();
            // One entry per name: `pz.click` delegates to the same op the helper
            // for it sends, and a doubled name is not a second action.
            if !name.is_empty() && !found.contains(&name) {
                found.push(name);
            }
            rest = &after[end..];
        }
        found
    }

    /// Both halves are present, and the walker is the one that defines the
    /// numbering every other file depends on.
    #[test]
    fn the_injection_carries_both_halves() {
        let injected = injection();
        assert!(injected.contains("__pzWalk"), "the walker is missing");
        assert!(injected.contains("window.pz"), "the API is missing");
    }

    /// A scan of the real source, so the test above is not vacuous.
    #[test]
    fn the_scan_finds_the_actions_that_are_definitely_there() {
        let found = ops_in(PRELUDE);
        for op in ["click", "insertText", "key", "wheel", "goto", "newTab"] {
            assert!(found.iter().any(|name| name == op), "{op} was not found");
        }
    }
}
