//! What the model is told about the machine it is working on.
//!
//! # Why this exists
//!
//! A model handed a filesystem tool has to guess what kind of machine it is on
//! and where its root is, and it guesses from whatever it has been trained on:
//! overwhelmingly a Linux container. Naming the Windows shell and its commands
//! prevents it from reaching for Unix syntax the terminal cannot run. Left to
//! inference it also invents roots -- `/app` and `/workspace` appear out of
//! nothing when the real answer is `F:\Vscode`.
//!
//! None of that is the model's fault. It is being asked to act on a machine it
//! cannot see, and the fix is to tell it rather than to expect it to guess.
//!
//! # Why it is not in the user's project instructions
//!
//! Project instructions are the user's to write and are sent verbatim. The
//! machine's shell and its root are facts about the app, not preferences, so they
//! are assembled here and prepended -- a model cannot be left to invent them
//! because the user did not think to mention them.
//!
//! # Why this costs nothing when it is absent
//!
//! A run with no tools does not need any of it, and the caller omits the message
//! entirely rather than sending a note about a filesystem the run cannot touch.
//! That keeps the prompt cache prefix identical to what it was before this
//! existed for every Chat-mode conversation.

/// One line naming the shell.
///
/// Platform and command syntax together, because the two are what a model
/// actually gets wrong. A one-line reminder is worth more than the tool
/// description being longer, because a model reading the schema will not infer
/// the platform from it.
pub fn shell_note() -> String {
    if cfg!(windows) {
        "The shell is Windows PowerShell 5.1 (`powershell.exe`), not cmd.exe or PowerShell 7. Use `;` to sequence commands; `&&` is not supported. \\
Use PowerShell commands such as `Get-ChildItem` (list files), `Get-Content` (read files), `Select-String` (search text), and `$env:USERNAME` (current user). Native programs such as `cargo`, `npm`, and `git` are available on PATH."
            .to_string()
    } else {
        "The shell is `sh`, not bash. `&&` chains commands, `&` backgrounds one, and standard \
Unix tools (`ls`, `cat`, `grep`) are available."
            .to_string()
    }
}

/// The workspace note: the folder relative paths resolve against.
///
/// `root` is `None` when no folder is open, and that is a case worth naming
/// explicitly rather than omitting: a model that knows a workspace exists but not
/// which one will guess a root, which is exactly the `/app` hallucination this
/// note exists to prevent.
///
/// The absolute root is named in the other case because the model cannot
/// otherwise discover it -- every tool takes a relative path and returns a
/// relative path, so nothing in a tool result would ever reveal where it was
/// operating.
pub fn workspace_note(root: Option<&str>) -> String {
    match root {
        Some(root) => format!(
            "Every relative path in these tools resolves against the workspace root, which is \
             `{root}`. Paths are always relative to it, may not be absolute, and may not contain \
             `..`. The terminal runs in this same folder, so a relative path means the same thing \
             to both."
        ),
        None => "No workspace folder is open, so these tools cannot read or write anything yet. \
                 Any filesystem or terminal tool you call will be refused. Tell the user to open a \
                 folder, and do not invent a path — there is no root to guess."
            .to_string(),
    }
}

/// The whole environment note, or nothing when the run has no tools.
///
/// Returning `None` for a tool-less run is the point: a Chat-mode request must
/// keep exactly the bytes it had, because that prefix is what a provider caches.
pub fn environment_note(has_tools: bool, root: Option<&str>) -> Option<String> {
    if !has_tools {
        return None;
    }
    Some(format!("{}\n\n{}", shell_note(), workspace_note(root)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Windows note has to name the actual shell and its command syntax.
    #[test]
    fn the_windows_note_names_powershell_and_its_syntax() {
        if cfg!(windows) {
            let note = shell_note();
            assert!(note.contains("PowerShell 5.1"), "{note}");
            assert!(note.contains("`powershell.exe`"), "{note}");
            assert!(note.contains("`&&` is not supported"), "{note}");
            assert!(note.contains("Get-ChildItem"), "{note}");
            assert!(note.contains("$env:USERNAME"), "{note}");
        }
    }

    /// Telling a model it is on PowerShell when it is on sh would be its own bug,
    /// so the note has to track the platform it is compiled for.
    #[test]
    fn the_shell_note_matches_the_platform_it_is_compiled_for() {
        let note = shell_note();
        if cfg!(windows) {
            assert!(note.contains("PowerShell"), "{note}");
        } else {
            assert!(note.contains("`sh`"), "{note}");
        }
    }

    /// The root is named explicitly, because nothing a tool returns would ever
    /// reveal it: every path in and out is relative.
    #[test]
    fn the_workspace_note_names_the_absolute_root() {
        let note = workspace_note(Some("F:\\Vscode"));
        assert!(note.contains("F:\\Vscode"), "{note}");
        // Both tools must be tied to the same root, which is the whole reason
        // the note exists: a model told one root and given another cannot tell.
        assert!(note.contains("terminal runs in this same folder"), "{note}");
    }

    /// With no folder open the note has to say so, and has to stop the model
    /// inventing a root. Silence here is what produced the `/app` claim: a model
    /// told a workspace exists but not which one will supply its own.
    #[test]
    fn an_unopened_workspace_says_so_and_forbids_guessing_a_root() {
        let note = workspace_note(None);
        assert!(note.contains("No workspace"), "{note}");
        assert!(note.contains("do not invent a path"), "{note}");
    }

    /// A Chat-mode run gets no note at all. Its system prompt has to stay
    /// byte-identical or the provider cache prefix is thrown away on every turn,
    /// which is the cost the prompt-caching notes in the README describe.
    #[test]
    fn a_run_with_no_tools_gets_no_environment_note() {
        assert_eq!(environment_note(false, Some("F:\\Vscode")), None);
        assert!(environment_note(true, Some("F:\\Vscode")).is_some());
    }
}
