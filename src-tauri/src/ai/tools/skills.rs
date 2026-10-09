//! Reading and managing stored skills.
//!
//! # Why the model can reach these
//!
//! The composer applies a skill the user checked, and that path sends the
//! instructions inline — reliable, because there is no decision for the model to
//! get wrong. This file is the other half: the skills the user *did not* check,
//! which the model may judge relevant from their names and descriptions and pull
//! itself. That is the progressive-disclosure pattern, and it is why the
//! descriptions a user writes on a skill matter as much as the body.
//!
//! # Why managing them is a separate tool
//!
//! Reading a skill is data. Creating, editing and deleting one changes stored
//! state, so it is `Effect::Write` and asks like any other write — which is the
//! whole point of putting it through the same gate rather than giving the model a
//! private path to the table.
//!
//! Two tools rather than one, because a single `skill(action, ...)` would make
//! every read carry a write-shaped permission and a write carry a read-shaped
//! description. The split lets the permission policy allow reading independently
//! from managing saved skills.

use serde_json::Value;

use crate::database::skills::{NewSkill, SkillOrigin};

use super::ToolSpec;

/// Lists the skills available and returns their full instructions.
///
/// Separate from the summary a model already has, because the bodies are the
/// expensive part and they are only worth sending once the model has decided one
/// is relevant.
pub const SKILL_READ: ToolSpec = ToolSpec {
    name: "skill_read",
    description: "Read a saved skill's full instructions. Call this before your first tool call \
                  or edit whenever the task matches a skill listed in your system prompt — \
                  writing, editing, reviewing, researching or drafting. Returns the instructions \
                  to follow.",
    parameters: r#"{
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "description": "The skill's name, exactly as listed."
            }
        },
        "required": ["name"],
        "additionalProperties": false
    }"#,
    // A read: the user already wrote these instructions and stored them, so
    // fetching one back is no more consequential than reading a file.
    effect: super::Effect::Read,
    command_argument: None,
    execute: |arguments, context| {
        Box::pin(std::future::ready(read(
            arguments,
            context.database.as_deref(),
        )))
    },
};

/// The catalogue of saved skills, as the model sees it.
///
/// **Names and descriptions only, never a body.** This is the whole point of the
/// index: a model can only decide a skill is relevant if it knows the skill
/// exists, and a body per skill would put every stored instruction into the
/// prompt of every run whether or not it applied. The description is the only text
/// it gets, so it is written as a trigger — "when to read this" — rather than a
/// summary of what is inside.
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
pub fn index(database: Option<&crate::database::Database>) -> Option<String> {
    let database = database?;
    let skills = database.enabled_skills().ok()?;
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
            line
        })
        .collect::<Vec<_>>()
        .join("\n");
    Some(format!(
        "You have skills: saved sets of instructions for how this user wants particular kinds of \
work done, listed below one per line as `name [type] — when to use it`. Read this list before \
your first action on every task. When a skill's name, type or description matches what you are \
about to do, call `skill_read` with its exact name and follow what it returns. Do this before you \
write or edit code, review, research or draft prose, and do not skip it because the task looks \
simple. When more than one applies, read each. If none apply, ignore this list.\n\n{listed}"
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

/// Creates, edits, enables or disables a skill.
///
/// Deliberately no delete. Deleting is the one operation with no undo, and a model
/// that can remove a thing the user wrote by hand is a much larger claim than one
/// that can stop it being applied — `set_enabled` already covers the reversible
/// half. The user deletes their own skills in Settings.
pub const SKILL_MANAGE: ToolSpec = ToolSpec {
    name: "skill_manage",
    description: "Save a reusable set of instructions as a skill, or update one that already \
                  exists. Use this when the user tells you how they want something done in a \
                  way worth remembering. Prefer updating an existing skill over creating a \
                  near-duplicate.",
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
                "description": "The full instructions to store, in the second person and specific enough to act on."
            },
            "enabled": {
                "type": "boolean",
                "description": "For update only. Set false to stop a skill being offered, keeping the text."
            }
        },
        "required": ["action", "instructions"],
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

/// The database, or the reason there isn't one.
///
/// A run with no session has no conversation to scope stored state to, and a
/// skill tool that guessed would be answering one conversation from another's
/// rows. The message names the fix rather than reporting a bare refusal.
fn require_database(
    database: Option<&crate::database::Database>,
) -> Result<&crate::database::Database, String> {
    database.ok_or_else(|| {
        "Skills are unavailable in this run because it has no conversation. \
         Ask the user to start a conversation and try again."
            .to_string()
    })
}

/// Turns the caller's arguments into the validated row shape.
///
/// Shared by both actions so `create` and `update` cannot disagree about what a
/// blank label or a missing description means — the difference between the two is
/// only which field identifies the skill.
///
/// `required` is the create case: only `create` must be told a name, because an
/// update carries the existing one in `existing_name` and a model fixing a
/// skill's wording should not have to restate its title to do so.
fn draft_from(arguments: &Value, required: bool) -> Result<NewSkill, String> {
    let text = |field: &str| -> Option<String> {
        arguments
            .get(field)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    // `existing_name` first when it is present: an update names the skill it is
    // replacing, and a `name` sent alongside it is a rename request rather than
    // the identifier. Taking `name` first would silently create a duplicate
    // whenever a model filled both in.
    let name = if required {
        text("name").ok_or("skill_manage create requires a `name`")?
    } else {
        text("name").or_else(|| text("existing_name")).ok_or(
            "skill_manage requires a `name`, or the `existing_name` of the skill to update",
        )?
    };
    Ok(NewSkill {
        name,
        description: text("description"),
        instructions: text("instructions")
            .ok_or("skill_manage requires `instructions` — the text to store")?,
        skill_type: text("skill_type"),
        // Always `Generated`: this skill was drafted by the model from a
        // conversation, and the schema carries an origin precisely so the user
        // can tell a deliberate one from an inferred one.
        origin: SkillOrigin::Generated,
    })
}

fn read(arguments: Value, database: Option<&crate::database::Database>) -> Result<String, String> {
    let database = require_database(database)?;
    let name = arguments
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("skill_read requires the `name` of a skill")?;

    let skill = database
        .enabled_skills()
        .map_err(|error| format!("Could not read the saved skills: {error}"))?
        .into_iter()
        .find(|skill| skill.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| {
            // Names rather than ids on purpose: the model never sees an id, so
            // asking for one would make the tool uncallable. The failure lists the
            // real names, because a model guessing at them is the likely cause.
            let available = database
                .enabled_skills()
                .map(|skills| {
                    skills
                        .iter()
                        .map(|skill| skill.name.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            format!("No skill named `{name}`. Available: {available}")
        })?;

    // A read is a use. The user watched the model choose this skill -- the
    // `skill_read` row is right there in the activity panel -- so the stored
    // count has to agree with what was on screen. The composer's checked skills
    // are counted on the reply path; this is the other half, and without it the
    // Skills screen undercounts every skill the model pulled for itself.
    database.record_skill_use(&skill.id);
    Ok(format!("# {}\n\n{}", skill.name, skill.instructions))
}

fn manage(
    arguments: Value,
    database: Option<&crate::database::Database>,
) -> Result<String, String> {
    let database = require_database(database)?;
    match arguments
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("")
    {
        "create" => {
            let skill = draft_from(&arguments, true)?;
            let created = database
                .create_skill(&skill)
                .map_err(|error| format!("Could not save the skill: {error}"))?;
            Ok(format!(
                "Saved the skill `{}`. It will be offered to the user from now on.",
                created.name
            ))
        }
        "update" => {
            let skill = draft_from(&arguments, false)?;
            // Identified by `existing_name` when sent, otherwise by the name the
            // update is writing. Both resolve to the same row in the common case,
            // and preferring `existing_name` is what makes a rename update rather
            // than a second skill.
            let target = arguments
                .get("existing_name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(skill.name.as_str());
            let existing = database
                .enabled_skills()
                .map_err(|error| format!("Could not read the saved skills: {error}"))?
                .into_iter()
                .find(|entry| entry.name.eq_ignore_ascii_case(target))
                .ok_or_else(|| {
                    let available = database
                        .enabled_skills()
                        .map(|skills| {
                            skills
                                .iter()
                                .map(|skill| skill.name.clone())
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default();
                    format!("No skill named `{target}`. Available: {available}")
                })?;

            // `enabled` is handled apart from the text because the update path
            // deliberately does not carry it: a save that looked like the same
            // request must not be able to silently switch a skill back on.
            if let Some(enabled) = arguments.get("enabled").and_then(Value::as_bool) {
                database
                    .set_skill_enabled(&existing.id, enabled)
                    .map_err(|error| format!("Could not change the skill: {error}"))?;
            }
            let updated = database
                .update_skill(&existing.id, &skill)
                .map_err(|error| format!("Could not save the skill: {error}"))?;
            Ok(format!("Updated the skill `{}`.", updated.name))
        }
        other => Err(format!(
            "`{other}` is not a skill_manage action. Use \"create\" or \"update\"."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn database() -> std::sync::Arc<crate::database::Database> {
        let database = std::sync::Arc::new(
            crate::database::Database::in_memory().expect("an in-memory database"),
        );
        database
            .create_skill(&NewSkill {
                name: "Rust house style".into(),
                description: Some("how Rust is written here".into()),
                instructions: "Prefer Result over panic.".into(),
                skill_type: None,
                origin: SkillOrigin::User,
            })
            .expect("seed");
        database
    }

    /// The index exists so a model can tell a skill is worth reading. Without the
    /// name it cannot ask, and without the description it cannot tell which.
    #[test]
    fn the_index_lists_names_and_descriptions() {
        let database = database();
        let text = index(Some(database.as_ref())).expect("an index");
        assert!(text.contains("Rust house style"), "{text}");
        assert!(text.contains("how Rust is written here"), "{text}");
    }

    /// A body in the index would put every stored instruction into every request.
    /// The whole design is that only the trigger text travels until one is chosen.
    #[test]
    fn the_index_carries_no_instructions() {
        let database = database();
        let text = index(Some(database.as_ref())).expect("an index");
        assert!(!text.contains("Prefer Result over panic."), "{text}");
    }

    /// `type` is named because it is the one grouping a skill has, and the only
    /// way the model can reason about a whole family -- "the code skills" --
    /// without reading a body.
    #[test]
    fn the_index_names_a_skills_type_when_it_has_one() {
        let database = std::sync::Arc::new(
            crate::database::Database::in_memory().expect("an in-memory database"),
        );
        database
            .create_skill(&NewSkill {
                name: "Rust house style".into(),
                description: Some("how Rust is written here".into()),
                instructions: "Prefer Result over panic.".into(),
                skill_type: Some("code".into()),
                origin: SkillOrigin::User,
            })
            .expect("seed");
        let text = index(Some(database.as_ref())).expect("an index");
        assert!(text.contains("Rust house style [code]"), "{text}");
        assert!(text.contains("— how Rust is written here"), "{text}");
    }

    /// A skill with no description still has to be listed: the model can still
    /// read it, and omitting it would make it unreachable rather than undescribed.
    #[test]
    fn a_skill_without_a_description_is_still_listed() {
        let database = std::sync::Arc::new(
            crate::database::Database::in_memory().expect("an in-memory database"),
        );
        database
            .create_skill(&NewSkill {
                name: "No blurb".into(),
                description: None,
                instructions: "Be brief.".into(),
                skill_type: None,
                origin: SkillOrigin::User,
            })
            .expect("seed");
        let text = index(Some(database.as_ref())).expect("an index");
        assert!(text.contains("No blurb"), "{text}");
    }

    /// Disabled skills are not offered, so they must not appear here either — an
    /// index listing a skill the model cannot read is a dead end.
    #[test]
    fn a_disabled_skill_is_not_offered() {
        let database = database();
        let disabled = database
            .list_skills()
            .expect("list")
            .into_iter()
            .find(|skill| skill.name == "Rust house style")
            .expect("seeded");
        database
            .set_skill_enabled(&disabled.id, false)
            .expect("disable");
        assert_eq!(index(Some(database.as_ref())), None);
    }

    /// An empty catalogue must produce no section at all. A paragraph explaining
    /// that there are no skills is a claim about the world, not an instruction.
    #[test]
    fn no_skills_produces_no_section() {
        let empty = crate::database::Database::in_memory().expect("an in-memory database");
        assert_eq!(index(Some(&empty)), None);
        // A run with no session has nothing to read a catalogue from.
        assert_eq!(index(None), None);
    }

    /// The model has to be told how to use the catalogue, or a list of names is
    /// just text it will not act on.
    #[test]
    fn the_index_says_how_to_use_it() {
        let database = database();
        let text = index(Some(database.as_ref())).expect("an index");
        assert!(text.contains("skill_read"), "{text}");
    }

    #[test]
    fn reading_a_skill_returns_its_instructions_and_name() {
        let database = database();
        let out = read(
            json!({ "name": "Rust house style" }),
            Some(database.as_ref()),
        )
        .expect("read");
        // The name travels with the body: a set of rules without its heading is
        // assertions with nothing to attach them to a topic.
        assert!(out.contains("Rust house style"), "{out}");
        assert!(out.contains("Prefer Result over panic."), "{out}");
    }

    /// Reading a skill counts as using it, so the Skills screen's figure matches
    /// the `skill_read` row the user already saw in the panel.
    #[test]
    fn reading_a_skill_counts_as_a_use() {
        let database = database();
        let before = use_count(&database, "Rust house style");
        read(
            json!({ "name": "Rust house style" }),
            Some(database.as_ref()),
        )
        .expect("read");
        assert_eq!(use_count(&database, "Rust house style"), before + 1);
    }

    /// A failed read resolves to no skill, so there is nothing to count -- and
    /// counting the attempt would make the figure rise on every typo.
    #[test]
    fn a_refused_read_counts_nothing() {
        let database = database();
        let before = use_count(&database, "Rust house style");
        assert!(read(json!({ "name": "nope" }), Some(database.as_ref())).is_err());
        assert_eq!(use_count(&database, "Rust house style"), before);
    }

    /// The stored use count for a skill, by name.
    fn use_count(database: &std::sync::Arc<crate::database::Database>, name: &str) -> i64 {
        database
            .enabled_skills()
            .expect("skills")
            .into_iter()
            .find(|skill| skill.name == name)
            .expect("seeded skill")
            .use_count
    }

    /// The model is never shown an id, so matching on one would make the tool
    /// uncallable in practice.
    #[test]
    fn a_skill_is_found_by_name_without_regard_to_case() {
        let database = database();
        let out = read(
            json!({ "name": "rust HOUSE style" }),
            Some(database.as_ref()),
        )
        .expect("read");
        assert!(out.contains("Prefer Result over panic."), "{out}");
    }

    /// A model guessing names is the likely failure, so the refusal carries the
    /// real ones rather than only saying no.
    #[test]
    fn an_unknown_name_says_what_is_available() {
        let database = database();
        let error = read(json!({ "name": "nope" }), Some(database.as_ref())).expect_err("no such");
        assert!(error.contains("Rust house style"), "{error}");
    }

    #[test]
    fn reading_without_a_name_is_reported() {
        let database = database();
        assert!(read(json!({}), Some(database.as_ref())).is_err());
    }

    /// A run with no conversation must not answer from stored state at all, since
    /// there is no conversation to scope it to.
    #[test]
    fn a_run_with_no_session_reports_rather_than_reading() {
        let error = read(json!({ "name": "Rust house style" }), None).expect_err("no database");
        assert!(error.contains("conversation"), "{error}");
    }

    #[test]
    fn creating_stores_the_instructions_as_a_generated_skill() {
        let database = database();
        manage(
            json!({
                "action": "create",
                "name": "Test checklist",
                "description": "what to check before calling something done",
                "instructions": "Run the tests. Read the diff."
            }),
            Some(database.as_ref()),
        )
        .expect("create");

        let stored = database
            .list_skills()
            .expect("list")
            .into_iter()
            .find(|skill| skill.name == "Test checklist")
            .expect("stored");
        assert_eq!(stored.instructions, "Run the tests. Read the diff.");
        // The origin is what lets the user tell a deliberate skill from one the
        // model inferred, and a tool-created skill is always inferred.
        assert_eq!(stored.origin, SkillOrigin::Generated);
    }

    /// Editing must not reset a skill's history. A save that looked like the same
    /// request resetting `use_count` would make editing look like recreating.
    #[test]
    fn updating_keeps_the_skills_identity() {
        let database = database();
        let original = database
            .list_skills()
            .expect("list")
            .into_iter()
            .find(|skill| skill.name == "Rust house style")
            .expect("seeded");
        database.record_skill_use(&original.id);

        manage(
            json!({
                "action": "update",
                "existing_name": "Rust house style",
                "name": "Rust house style",
                "instructions": "Prefer pure functions."
            }),
            Some(database.as_ref()),
        )
        .expect("update");

        let stored = database
            .list_skills()
            .expect("list")
            .into_iter()
            .find(|skill| skill.name == "Rust house style")
            .expect("still there");
        assert_eq!(stored.id, original.id);
        assert_eq!(stored.use_count, 1);
        assert_eq!(stored.instructions, "Prefer pure functions.");
    }

    #[test]
    fn updating_an_unknown_skill_is_refused_and_says_what_exists() {
        let database = database();
        let error = manage(
            json!({ "action": "update", "existing_name": "nope", "instructions": "x" }),
            Some(database.as_ref()),
        )
        .expect_err("no such skill");
        assert!(error.contains("Rust house style"), "{error}");
    }

    #[test]
    fn an_unknown_action_is_refused_by_name() {
        let database = database();
        let error = manage(
            json!({ "action": "destroy", "name": "x", "instructions": "y" }),
            Some(database.as_ref()),
        )
        .expect_err("bad action");
        assert!(error.contains("destroy"), "{error}");
    }

    /// A model fixing a skill's wording should not have to restate its title, and
    /// blanking the name would fail validation for a reason it cannot see.
    #[test]
    fn an_update_without_a_name_keeps_the_existing_one() {
        let database = database();
        manage(
            json!({ "action": "update", "existing_name": "Rust house style", "instructions": "Prefer small functions." }),
            Some(database.as_ref()),
        )
        .expect("update");

        let stored = database
            .skill(
                &database
                    .list_skills()
                    .expect("list")
                    .into_iter()
                    .find(|skill| skill.name == "Rust house style")
                    .expect("still there")
                    .id,
            )
            .expect("read")
            .expect("row");
        assert_eq!(stored.instructions, "Prefer small functions.");
    }

    /// A skill with no instructions is refused by the schema's own validation, and
    /// the message says so rather than reporting a constraint the model cannot see.
    #[test]
    fn instructions_are_required() {
        let database = database();
        let error = manage(
            json!({ "action": "create", "name": "Empty" }),
            Some(database.as_ref()),
        )
        .expect_err("no instructions");
        assert!(error.contains("instructions"), "{error}");
    }

    #[test]
    fn a_created_skill_is_readable_by_name_afterwards() {
        let database = database();
        manage(
            json!({ "action": "create", "name": "Fresh", "instructions": "Be brief." }),
            Some(database.as_ref()),
        )
        .expect("create");
        let out = read(json!({ "name": "Fresh" }), Some(database.as_ref())).expect("read");
        assert!(out.contains("Be brief."), "{out}");
    }
}
