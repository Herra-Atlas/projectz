//! Reading, writing and running folder-backed skills.
//!
//! # Why the model can reach these
//!
//! A skill is a folder -- `SKILL.md` plus whatever `scripts/`, `references/` and
//! `assets/` it carries -- and the model sees only an index of names and
//! triggers. [`SKILL_READ`] pulls one body when it decides the task matches;
//! [`SKILL_WRITE`] lets it add a file to a skill it is working on; [`SKILL_RUN`]
//! executes a script the user has trusted. That is progressive disclosure: the
//! index is cheap, the body costs a read, and a script costs a process.
//!
//! # Why running is separate, and why trust is checked here
//!
//! Reading a skill is data. Creating, editing or removing one changes what the
//! app will offer later, so it is `Effect::Write`. Running a script is arbitrary
//! code, so it is also a write -- but the extra gate is not the permission level,
//! it is the per-skill `trusted` flag. A skill the agent just drafted, or one
//! installed from a link, is readable and useful while its scripts stay inert.
//! That is the line that keeps `skill_manage` from being a code-execution
//! primitive.
//!
//! # Where the folders are
//!
//! Outside the workspace, deliberately: the workspace is the user's project, a
//! skill is the app's material, and `read_file` must not be widened to reach it.
//! Every path argument here resolves inside one skill folder using the same
//! containment check the file tools use.

use std::path::Path;

use serde_json::Value;

use crate::database::Database;

use super::{ToolContext, ToolSpec};

/// Lists the skills available and returns one's full instructions.
pub const SKILL_READ: ToolSpec = ToolSpec {
    name: "skill_read",
    description: "Read a saved skill's full instructions. Call this when the task matches a skill \
                  listed in your system prompt — writing, editing, reviewing, debugging, testing, \
                  researching or drafting prose — and follow what it returns rather than working \
                  the approach out from scratch. Omit `path` for SKILL.md; pass a path from the \
                  file list it prints to read one reference, script or asset. Returns the text.",
    parameters: r#"{
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "description": "The skill's name, as listed. The whole `name [type]` label works too."
            },
            "path": {
                "type": "string",
                "description": "Optional. A supporting file inside that skill, for example `references/REFERENCE.md` or `scripts/run.ps1`. Omit to read the skill's own instructions."
            }
        },
        "required": ["name"],
        "additionalProperties": false
    }"#,
    effect: super::Effect::Read,
    command_argument: None,
    execute: |arguments, context| {
        Box::pin(std::future::ready(read(
            arguments,
            context.database.as_deref(),
        )))
    },
};

/// The catalogue of skills, as the model sees it.
///
/// **Names and triggers only, never a body.** This is the whole point of the
/// index: a model can only decide a skill is relevant if it knows the skill
/// exists, and a body per skill would put every stored instruction into the
/// prompt of every run whether or not it applied. The description is the only
/// text it gets, so it is written as a trigger -- "when to read this" -- rather
/// than a summary of what is inside.
///
/// **Built per request from the enabled set, and constant across a conversation.**
/// Reading a skill is a tool call, which grows the conversation rather than this
/// prefix, so the catalogue does not move when one is applied. Only editing or
/// disabling a skill changes it, which is the honest cost of the index being here
/// at all.
///
/// `None` when there are no skills, or when the run has no session to read them
/// for. Returning an empty string instead would put a paragraph about having no
/// skills into every request, which is a claim about the world rather than an
/// instruction.
pub fn index(database: Option<&Database>) -> Option<String> {
    let database = database?;
    let skills = crate::skills::enabled(database).ok()?;
    if skills.is_empty() {
        return None;
    }
    let listed = skills
        .iter()
        .map(|skill| {
            // `type` is the one grouping a skill has, and the only way the model
            // can reason about a whole family -- "the code skills" -- without
            // reading a body. Bracketed so it reads as a label rather than part
            // of the name, and omitted when the skill carries none.
            let mut line = format!("- {}", skill.name);
            if let Some(label) = labelled(&skill.skill_type) {
                line.push_str(&format!(" [{label}]"));
            }
            if let Some(description) = labelled(&skill.description) {
                line.push_str(&format!(" — {description}"));
            }
            if skill.file_count > 0 {
                line.push_str(&format!(
                    " ({} file{})",
                    skill.file_count,
                    plural(skill.file_count)
                ));
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n");
    Some(format!(
        "You have skills: folders of instructions and supporting files for how this user wants \
particular kinds of work done, listed below one per line as `name [type] — when to use it`. \
Before you start, check this list and call `skill_read` with the exact name of the skill whose \
name, type or description matches what you are about to do -- writing or editing code, \
reviewing, debugging, testing, researching, drafting prose -- then follow what it returns. The \
result lists the files in the skill; read one with `skill_read` and `path` only if the \
instructions point at it, and run a script with `skill_run` only when the skill says to. Read the \
closest match: one skill is usually the right number, and each further one costs a round trip. If \
none apply, ignore this list and get on with the task.\n\n{listed}"
    ))
}

/// A trimmed, non-empty optional field as a slice, or nothing.
///
/// The catalogue skips a blank label rather than printing `[]` or a dangling
/// separator, so an unlabelled or description-less skill still reads as one line.
fn labelled(value: &Option<String>) -> Option<&str> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

/// Creates or edits a skill's own text.
///
/// Deliberately no delete. Deleting is the one operation with no undo, and a
/// model that can remove a folder the user wrote by hand is a much larger claim
/// than one that can stop it being applied — `enabled: false` already covers the
/// reversible half. The user deletes their own skills in Settings.
pub const SKILL_MANAGE: ToolSpec = ToolSpec {
    name: "skill_manage",
    description: "Save a reusable set of instructions as a skill, or update one that already \
                  exists. Use this when the user tells you how they want something done in a \
                  way worth remembering. Prefer updating an existing skill over creating a \
                  near-duplicate. This writes SKILL.md only; use `skill_write` for scripts and \
                  references.",
    parameters: r#"{
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "enum": ["create", "update"],
                "description": "Create a new skill, or replace the text of an existing one."
            },
            "name": {
                "type": "string",
                "description": "The skill's name, as shown in the list. Required for create, and for update when renaming."
            },
            "existing_name": {
                "type": "string",
                "description": "The current name of the skill to update. Use this for update, so a rename does not create a second skill."
            },
            "description": {
                "type": "string",
                "description": "One line saying when this skill applies. This is the only text a model sees before deciding to read the skill, so write it as a trigger rather than a summary."
            },
            "instructions": {
                "type": "string",
                "description": "The full SKILL.md body, in the second person and specific enough to act on. Markdown."
            },
            "skill_type": {
                "type": "string",
                "description": "One word naming the family this skill belongs to, shown in brackets beside its name in the list. Use \"unorganized\" when none fits."
            },
            "enabled": {
                "type": "boolean",
                "description": "For update only. Set false to stop a skill being offered, keeping the text."
            }
        },
        "required": ["action", "instructions", "skill_type"],
        "additionalProperties": false
    }"#,
    // A write: it changes stored state the user will rely on later.
    effect: super::Effect::Write,
    command_argument: None,
    execute: |arguments, context| {
        Box::pin(std::future::ready(manage(
            arguments,
            context.database.as_deref(),
        )))
    },
};

/// Writes a supporting file into a skill, or copies an asset from the workspace.
pub const SKILL_WRITE: ToolSpec = ToolSpec {
    name: "skill_write",
    description: "Add or replace a file inside a skill: a script under `scripts/`, a reference \
                  under `references/`, or a template under `assets/`. Pass `content` for text, or \
                  `from_path` to copy an existing workspace file (an image, a PDF, a template) \
                  byte for byte. SKILL.md itself is edited with `skill_manage`.",
    parameters: r#"{
        "type": "object",
        "properties": {
            "skill": {
                "type": "string",
                "description": "The skill's name or slug, as listed."
            },
            "path": {
                "type": "string",
                "description": "Destination inside the skill, for example `scripts/run.ps1` or `references/CHECKLIST.md`. Must be relative."
            },
            "content": {
                "type": "string",
                "description": "The file's text. Omit when using `from_path`."
            },
            "from_path": {
                "type": "string",
                "description": "A file in the workspace to copy in, relative to the workspace root. Use this for anything binary."
            }
        },
        "required": ["skill", "path"],
        "additionalProperties": false
    }"#,
    // A write: it grows what the skill can do, and invalidates earlier reads.
    effect: super::Effect::Write,
    command_argument: None,
    execute: |arguments, context| {
        Box::pin(std::future::ready(write(
            arguments,
            context.database.as_deref(),
        )))
    },
};

/// Runs one script from a skill the user has trusted.
pub const SKILL_RUN: ToolSpec = ToolSpec {
    name: "skill_run",
    description: "Run a script from a skill, with the skill's folder as the working directory. \
                  Use this when the skill's instructions say to run one. The skill must be marked \
                  trusted by the user; if it is not, tell the user to allow scripts on the skill's \
                  page rather than trying again. Returns the exit code and the command's output.",
    parameters: r#"{
        "type": "object",
        "properties": {
            "skill": {
                "type": "string",
                "description": "The skill's name or slug, as listed."
            },
            "script": {
                "type": "string",
                "description": "The script's path inside the skill, for example `scripts/extract.py`."
            },
            "args": {
                "type": "array",
                "description": "Arguments passed to the script, in order.",
                "items": { "type": "string" }
            }
        },
        "required": ["skill", "script"],
        "additionalProperties": false
    }"#,
    // Running a program can change anything, exactly like `run_terminal`.
    effect: super::Effect::Write,
    // Named so the approval prompt shows the script that will run, not just the
    // skill's name.
    command_argument: Some("script"),
    execute: |arguments, context| Box::pin(run(arguments, context)),
};

/// The database, or the reason there isn't one.
///
/// A run with no session has no conversation to scope stored state to, and a
/// skill tool that guessed would be answering one conversation from another's
/// rows. The message names the fix rather than reporting a bare refusal.
fn require_database(database: Option<&Database>) -> Result<&Database, String> {
    database.ok_or_else(|| {
        "Skills are unavailable in this run because it has no conversation. \
         Ask the user to start a conversation and try again."
            .to_string()
    })
}

/// Turns the caller's arguments into the validated draft shape.
///
/// `required` is the create case: only `create` must be told a name, because an
/// update carries the existing one in `existing_name` and a model fixing a
/// skill's wording should not have to restate its title to do so.
fn draft_from(arguments: &Value, required: bool) -> Result<crate::skills::SkillDraft, String> {
    let text = |field: &str| -> Option<String> {
        arguments
            .get(field)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let name = if required {
        text("name").ok_or("skill_manage create requires a `name`")?
    } else {
        text("name").or_else(|| text("existing_name")).ok_or(
            "skill_manage requires a `name`, or the `existing_name` of the skill to update",
        )?
    };
    Ok(crate::skills::SkillDraft {
        name,
        description: text("description"),
        instructions: text("instructions")
            .ok_or("skill_manage requires `instructions` — the text to store")?,
        skill_type: text("skill_type").or_else(|| Some(DEFAULT_TYPE.to_string())),
    })
}

/// The type a skill gets when the caller names none.
///
/// `type` is printed beside every skill in the index, so a skill without one
/// reads as an unlabelled row -- and the index is the only thing a model sees
/// before it decides whether to read a skill. The word is deliberately dull: it
/// is a placeholder for "this one did not fit a family", not a category worth
/// keeping, so nothing treats it as meaningful and the next save replaces it.
const DEFAULT_TYPE: &str = "unorganized";

/// The names a caller's request may match, most literal first.
///
/// The index lists a skill as `name [type] — when to use it`, and a model reading
/// it sometimes asks for that whole label as though it were the name. The
/// bracketed part is the type, which the model was told about precisely so it
/// could reason about a family of skills, so mistaking it for part of the name is
/// a reasonable mistake -- and one this tool can absorb rather than report, which
/// costs a round trip and teaches nothing.
fn names_to_try(requested: &str) -> Vec<&str> {
    let trimmed = requested.trim();
    let mut names = vec![trimmed];
    if let Some(start) = trimmed.find('[') {
        if trimmed.ends_with(']') && start > 0 {
            let bare = trimmed[..start].trim();
            if !bare.is_empty() {
                names.push(bare);
            }
        }
    }
    names
}

fn read(arguments: Value, database: Option<&Database>) -> Result<String, String> {
    let database = require_database(database)?;
    let name = arguments
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("skill_read requires the `name` of a skill")?;
    // `names_to_try` is kept as a shaping step rather than folded into resolve:
    // the folder resolver matches a slug or a label, and the bracketed label is
    // the one form only this tool has to absorb.
    let mut last = None;
    let mut skill = None;
    for candidate in names_to_try(name) {
        match crate::skills::resolve(database, candidate) {
            Ok(found) => {
                skill = Some(found);
                break;
            }
            Err(error) => last = Some(error),
        }
    }
    let skill = skill.ok_or_else(|| last.unwrap_or_else(|| format!("No skill named `{name}`")))?;

    if let Some(path) = arguments
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        let body = crate::skills::read_file(database, &skill.slug, path)?;
        database.record_skill_use(&skill.slug);
        return Ok(format!("# {}\n\n[{path}]\n\n{body}", skill.name));
    }

    let detail = crate::skills::get(database, &skill.slug)?;
    // A read is a use. The user watched the model choose this skill -- the
    // `skill_read` row is right there in the activity panel -- so the stored
    // count has to agree with what was on screen.
    database.record_skill_use(&skill.slug);
    let mut out = format!("# {}\n\n{}", detail.skill.name, detail.instructions.trim());
    if !detail.files.is_empty() {
        out.push_str("\n\nFiles in this skill (read one with `skill_read` and `path`):\n");
        for file in &detail.files {
            out.push_str(&format!(
                "- {} ({} bytes{})\n",
                file.path,
                file.size,
                if file.is_script { ", script" } else { "" }
            ));
        }
    }
    Ok(out)
}

fn manage(arguments: Value, database: Option<&Database>) -> Result<String, String> {
    let database = require_database(database)?;
    match arguments
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("")
    {
        "create" => {
            let draft = draft_from(&arguments, true)?;
            let created =
                crate::skills::create(database, &draft, crate::skills::SkillOrigin::Generated)?;
            Ok(format!(
                "Saved the skill `{}` as `{}`. It will be offered to the user from now on.",
                created.name, created.slug
            ))
        }
        "update" => {
            let draft = draft_from(&arguments, false)?;
            let target = arguments
                .get("existing_name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(draft.name.as_str());
            let existing = crate::skills::resolve(database, target)?;
            // `enabled` is handled apart from the text because the update path
            // deliberately does not carry it: a save that looked like the same
            // request must not be able to silently switch a skill back on.
            if let Some(enabled) = arguments.get("enabled").and_then(Value::as_bool) {
                database.set_skill_enabled(&existing.slug, enabled)?;
            }
            let updated = crate::skills::update(database, &existing.slug, &draft)?;
            Ok(format!("Updated the skill `{}`.", updated.name))
        }
        other => Err(format!(
            "`{other}` is not a skill_manage action. Use \"create\" or \"update\"."
        )),
    }
}

fn write(arguments: Value, database: Option<&Database>) -> Result<String, String> {
    let database = require_database(database)?;
    let skill = arguments
        .get("skill")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("skill_write requires the `skill` name")?;
    let path = arguments
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("skill_write requires a `path` inside the skill")?;
    let content = arguments.get("content").and_then(Value::as_str);
    let from_path = arguments
        .get("from_path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let file = match (content, from_path) {
        (Some(_), Some(_)) => {
            return Err("Pass either `content` or `from_path`, not both".to_string())
        }
        (Some(content), None) => crate::skills::write_file(database, skill, path, content)?,
        (None, Some(source)) => crate::skills::copy_file(database, skill, path, source)?,
        (None, None) => {
            return Err(
                "skill_write needs `content`, or `from_path` to copy a workspace file".to_string(),
            )
        }
    };
    Ok(format!(
        "Wrote `{}` ({} bytes) to the skill `{}`.{}",
        file.path,
        file.size,
        skill,
        if file.is_script {
            " Its scripts become runnable only after the user allows them on the skill's page."
        } else {
            ""
        }
    ))
}

async fn run(arguments: Value, context: ToolContext) -> Result<String, String> {
    let database = context.database.as_deref().ok_or_else(|| {
        "Skills are unavailable in this run because it has no conversation".to_string()
    })?;
    let name = arguments
        .get("skill")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("skill_run requires the `skill` name")?;
    let script = arguments
        .get("script")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("skill_run requires the `script` path")?;
    let args = arguments
        .get("args")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let (skill, path) = crate::skills::script_path(database, name, script)?;
    if !skill.enabled {
        return Err(format!(
            "The skill `{}` is turned off, so its scripts cannot run. Ask the user to enable it.",
            skill.name
        ));
    }
    if !crate::skills::trusted(database, &skill.slug)? {
        return Err(format!(
            "The skill `{}` has not been allowed to run scripts. Ask the user to turn on \
             \"Allow scripts\" on the skill's page, then try again.",
            skill.name
        ));
    }

    let skill_dir = crate::skills::directory_of(&skill)?;
    execute_script(&path, &skill_dir, &args, &context).await
}

/// Spawns one script with the skill folder as its working directory.
///
/// **The working directory is the skill root, not the script's own folder.** A
/// script that says `assets/input.pdf` means the skill's `assets/`, and a `cwd`
/// of `scripts/` would make every relative path in the instructions wrong. The
/// script is passed by its absolute path so it runs from either place.
///
/// Interpreter by extension rather than sniffing the shebang: this is a Windows
/// app, a shebang means nothing to PowerShell, and a file with no known extension
/// cannot be run at all. `SKILL_DIR` and `PROJECTZ_WORKSPACE` are exported so a
/// script can find its own assets and write results back to the user's project
/// without being told either path in its arguments.
async fn execute_script(
    script: &Path,
    skill_dir: &Path,
    args: &[String],
    context: &ToolContext,
) -> Result<String, String> {
    let extension = script
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    let (mut command, interpreter) = match extension.as_str() {
        "ps1" => {
            let mut command = tokio::process::Command::new("powershell.exe");
            command.args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ]);
            command.arg(script);
            (command, "powershell.exe")
        }
        "py" => {
            let mut command = tokio::process::Command::new("python");
            command.arg(script);
            (command, "python")
        }
        "js" | "mjs" | "cjs" => {
            let mut command = tokio::process::Command::new("node");
            command.arg(script);
            (command, "node")
        }
        "sh" | "bash" => {
            let mut command = tokio::process::Command::new("bash");
            command.arg(script);
            (command, "bash")
        }
        "cmd" | "bat" => {
            let mut command = tokio::process::Command::new("cmd");
            command.arg("/c").arg(script);
            (command, "cmd")
        }
        "rb" => {
            let mut command = tokio::process::Command::new("ruby");
            command.arg(script);
            (command, "ruby")
        }
        "pl" => {
            let mut command = tokio::process::Command::new("perl");
            command.arg(script);
            (command, "perl")
        }
        "exe" => (tokio::process::Command::new(script), "the file itself"),
        other => {
            return Err(format!(
                "`.{other}` is not a script this app knows how to run. Use .ps1, .py, .js, .sh, \
                 .cmd or .bat."
            ))
        }
    };
    for argument in args {
        command.arg(argument);
    }
    command
        .current_dir(skill_dir)
        .env("SKILL_DIR", skill_dir)
        .env("NO_COLOR", "1");
    if let Some(workspace) = super::workspace::root() {
        command.env("PROJECTZ_WORKSPACE", workspace);
    }
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    crate::child::hide_console_async(&mut command);

    let child = command.spawn().map_err(|error| {
        // A missing interpreter is the commonest failure and the least
        // informative as a raw OS error ("program not found"). Name the program
        // the script needed so the fix is obvious.
        if error.kind() == std::io::ErrorKind::NotFound {
            format!(
                "Could not run `{}`: {interpreter} is not installed or not on PATH.",
                script.display()
            )
        } else {
            format!("Could not start `{}`: {error}", script.display())
        }
    })?;
    let wait = child.wait_with_output();
    tokio::pin!(wait);
    let output = tokio::select! {
        finished = &mut wait => finished.map_err(|error| format!("The script failed: {error}"))?,
        _ = super::approval::wait_until_cancelled(&context.cancelled) => {
            return Err("Cancelled before the script finished".into());
        }
        _ = tokio::time::sleep(std::time::Duration::from_secs(super::terminal::MAX_RUNTIME_SECS)) => {
            return Err(format!(
                "The script was stopped after {}s without finishing. Run the long part in the \
                 background with run_terminal if it needs longer.",
                super::terminal::MAX_RUNTIME_SECS
            ));
        }
    };
    Ok(format!(
        "Exit code: {}\n\n{}{}",
        output.status.code().unwrap_or(-1),
        super::terminal::bounded_output(&output.stdout, &output.stderr),
        if output.status.success() {
            ""
        } else {
            "\n[The script reported a failure.]"
        }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The index prints `name [type]`; a model that copies the whole label must
    /// still find the skill.
    #[test]
    fn a_bracketed_label_is_tried_both_ways() {
        assert_eq!(
            names_to_try("Documentation [writing]"),
            vec!["Documentation [writing]", "Documentation"]
        );
        assert_eq!(names_to_try("Foo [bar]").first(), Some(&"Foo [bar]"));
        assert_eq!(names_to_try("  Email inbox  "), vec!["Email inbox"]);
        assert_eq!(names_to_try("[leading]"), vec!["[leading]"]);
        assert_eq!(names_to_try("unclosed [bracket"), vec!["unclosed [bracket"]);
    }

    /// A skill saved without a type is stored under the name for "no family",
    /// because the index prints a type beside every name and an unlabelled row is
    /// the one thing a model cannot judge.
    #[test]
    fn a_missing_type_becomes_unorganized() {
        let draft = draft_from(
            &json!({ "action": "create", "name": "Winding down", "instructions": "Say what changed." }),
            true,
        )
        .expect("draft");
        assert_eq!(draft.skill_type.as_deref(), Some(DEFAULT_TYPE));
    }

    /// A model fixing a skill's wording should not have to restate its title.
    #[test]
    fn an_update_can_name_the_skill_it_replaces() {
        let draft = draft_from(
            &json!({ "action": "update", "existing_name": "Rust house style", "instructions": "Be brief." }),
            false,
        )
        .expect("draft");
        assert_eq!(draft.name, "Rust house style");
    }

    #[test]
    fn an_empty_name_or_body_is_refused() {
        assert!(draft_from(&json!({ "name": "x" }), true).is_err());
        assert!(draft_from(&json!({ "instructions": "x" }), true).is_err());
    }

    /// The tools are classified for caching and approval: reads are cached,
    /// writes are not, and running a script carries a command the prompt can show.
    #[test]
    fn the_tools_have_the_effects_the_gate_expects() {
        assert_eq!(SKILL_READ.effect, super::super::Effect::Read);
        assert_eq!(SKILL_MANAGE.effect, super::super::Effect::Write);
        assert_eq!(SKILL_WRITE.effect, super::super::Effect::Write);
        assert_eq!(SKILL_RUN.effect, super::super::Effect::Write);
        assert_eq!(SKILL_RUN.command_argument, Some("script"));
        assert_eq!(SKILL_READ.command_argument, None);
    }
}
