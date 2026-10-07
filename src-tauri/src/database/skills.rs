//! Stored instructions the agent can be asked to apply.
//!
//! A skill is a named piece of instructions -- a house style, a review
//! checklist, a preference about how this project is written -- that gets folded
//! into the request for a reply that uses it.
//!
//! # This module is the schema and its queries only
//!
//! How a skill reaches a model is [`crate::ai::remote::client`]'s problem, not
//! this file's. The instructions travel inside the message list, built by the
//! frontend from the same `skills_enabled` list it offered them from and stored in
//! the transcript like any other message; Rust reads `skill_ids` from the request
//! only to keep `use_count` honest. Which skills apply to a conversation is the
//! user's decision, made in the composer, so nothing here has to decide it.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::database::Database;

/// Who wrote a skill.
///
/// Recorded because the three are trusted differently: a bundled one and a
/// user-written one were both written deliberately, while an agent-drafted one
/// was inferred from a past conversation and may encode an assumption that was
/// never checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillOrigin {
    /// Written deliberately by the user.
    User,
    /// Drafted by the agent from a past conversation.
    Generated,
    /// Shipped with the app, and editable like any other skill.
    Bundled,
}

impl Default for SkillOrigin {
    fn default() -> Self {
        Self::User
    }
}

impl SkillOrigin {
    /// The stored spelling.
    ///
    /// Round-tripped through the database rather than derived from serde, so a
    /// value written by either side reads back the same way.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Generated => "generated",
            Self::Bundled => "bundled",
        }
    }

    /// Reads a stored value, defaulting to `User`.
    ///
    /// The default is the conservative direction: a skill whose origin cannot be
    /// read is treated as deliberately written rather than machine-inferred, so
    /// an unreadable row is not presented to the user as its own work.
    pub fn from_str(value: &str) -> Self {
        match value {
            "generated" => Self::Generated,
            "bundled" => Self::Bundled,
            _ => Self::User,
        }
    }
}

/// One stored skill, as the frontend sees it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Skill {
    pub id: String,
    pub name: String,
    /// One line for the picker. Optional, because a skill whose name says enough
    /// should not need one written.
    pub description: Option<String>,
    pub instructions: String,
    /// Free-text label. `None` is unlabelled and is distinct from `Some("")`.
    pub skill_type: Option<String>,
    pub origin: SkillOrigin,
    pub enabled: bool,
    /// How many replies have used it. Maintained here rather than counted from a
    /// join, so the "most used" ordering costs one indexed read.
    pub use_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

/// Everything needed to create or replace a skill.
///
/// Separate from [`Skill`] so a caller cannot set `use_count` or `created_at` by
/// accident: those are maintained by the application, and an id and timestamps
/// are generated rather than accepted.
#[derive(Clone, Debug, Deserialize)]
pub struct NewSkill {
    pub name: String,
    pub description: Option<String>,
    pub instructions: String,
    pub skill_type: Option<String>,
    #[serde(default)]
    pub origin: SkillOrigin,
}

/// Why a write was refused.
///
/// A validation failure the caller can act on, rather than a bare string, so the
/// frontend can point at the field that is wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkillError {
    EmptyName,
    EmptyInstructions,
    TooLong(&'static str, usize, usize),
}

impl std::fmt::Display for SkillError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyName => write!(formatter, "A skill needs a name"),
            Self::EmptyInstructions => {
                write!(formatter, "A skill needs instructions to apply")
            }
            Self::TooLong(field, actual, limit) => write!(
                formatter,
                "`{field}` is {actual} characters; the limit is {limit}"
            ),
        }
    }
}

/// Longest a name may be, in characters.
///
/// Generous enough for a descriptive title and short enough to render in the
/// picker without truncating.
const MAX_NAME_CHARS: usize = 80;
const MAX_DESCRIPTION_CHARS: usize = 300;
const MAX_INSTRUCTIONS_CHARS: usize = 8_000;

impl NewSkill {
    /// Checks the fields a caller controls.
    ///
    /// Trimmed before measuring, so a name of ten spaces is empty rather than
    /// being stored and shown as a blank row.
    pub fn validate(&self) -> Result<(), SkillError> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(SkillError::EmptyName);
        }
        if name.chars().count() > MAX_NAME_CHARS {
            return Err(SkillError::TooLong(
                "name",
                name.chars().count(),
                MAX_NAME_CHARS,
            ));
        }
        if let Some(description) = &self.description {
            if description.trim().chars().count() > MAX_DESCRIPTION_CHARS {
                return Err(SkillError::TooLong(
                    "description",
                    description.trim().chars().count(),
                    MAX_DESCRIPTION_CHARS,
                ));
            }
        }
        if self.instructions.trim().is_empty() {
            return Err(SkillError::EmptyInstructions);
        }
        if self.instructions.chars().count() > MAX_INSTRUCTIONS_CHARS {
            return Err(SkillError::TooLong(
                "instructions",
                self.instructions.chars().count(),
                MAX_INSTRUCTIONS_CHARS,
            ));
        }
        Ok(())
    }
}

/// Trims the caller-controlled fields once, at the boundary.
///
/// Doing it here rather than at every read means a skill can never be displayed
/// with the whitespace a model produced around it.
fn normalized(skill: &NewSkill) -> NewSkill {
    NewSkill {
        name: skill.name.trim().to_string(),
        description: skill
            .description
            .as_ref()
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty()),
        instructions: skill.instructions.trim().to_string(),
        skill_type: skill
            .skill_type
            .as_ref()
            .map(|label| label.trim().to_string())
            .filter(|label| !label.is_empty()),
        origin: skill.origin,
    }
}

impl Database {
    /// Every skill, enabled or not, newest name order.
    pub fn list_skills(&self) -> Result<Vec<Skill>, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let mut statement = connection
            .prepare("SELECT id,name,description,instructions,type,origin,enabled,use_count,created_at,updated_at FROM skills ORDER BY enabled DESC, name COLLATE NOCASE")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], skill_from_row)
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    /// Enabled skills only, which is what a reply may use.
    ///
    /// Ordered by name rather than by use count: a reply picks from a list the
    /// user is choosing between, and an alphabetical list is one they can scan.
    /// The usage ranking belongs on the management screen, not here.
    pub fn enabled_skills(&self) -> Result<Vec<Skill>, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let mut statement = connection
            .prepare("SELECT id,name,description,instructions,type,origin,enabled,use_count,created_at,updated_at FROM skills WHERE enabled=1 ORDER BY name COLLATE NOCASE")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], skill_from_row)
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    /// One skill by id, or `None` when it does not exist.
    pub fn skill(&self, id: &str) -> Result<Option<Skill>, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        connection
            .query_row(
                "SELECT id,name,description,instructions,type,origin,enabled,use_count,created_at,updated_at FROM skills WHERE id=?1",
                [id],
                skill_from_row,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    /// Creates a skill, generating its id and timestamps.
    ///
    /// Insert-only. Replacing a skill is [`Database::update_skill`], which
    /// preserves `use_count`; doing it here would silently reset a skill's
    /// history every time a caller sent what looked like the same request.
    pub fn create_skill(&self, skill: &NewSkill) -> Result<Skill, String> {
        skill.validate().map_err(|error| error.to_string())?;
        let normalized = normalized(skill);
        let id = uuid::Uuid::new_v4().to_string();
        let now = crate::database::utc_now();

        self.connection
            .lock()
            .map_err(|error| error.to_string())?
            .execute(
                "INSERT INTO skills (id,name,description,instructions,type,origin,enabled,use_count,created_at,updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,1,0,?7,?7)",
                params![
                    id,
                    normalized.name,
                    normalized.description,
                    normalized.instructions,
                    normalized.skill_type,
                    normalized.origin.as_str(),
                    now,
                ],
            )
            .map_err(|error| error.to_string())?;

        self.skill(&id)?
            .ok_or_else(|| format!("Skill {id} vanished immediately after being written"))
    }

    /// Replaces the editable fields of a skill, keeping its id and usage.
    ///
    /// `name`, `description`, `instructions` and `type` come from the caller;
    /// `origin`, `enabled` and `use_count` do not, because they describe how the
    /// skill is used rather than what it says.
    pub fn update_skill(&self, id: &str, skill: &NewSkill) -> Result<Skill, String> {
        skill.validate().map_err(|error| error.to_string())?;
        let normalized = normalized(skill);
        let now = crate::database::utc_now();

        let changed = self
            .connection
            .lock()
            .map_err(|error| error.to_string())?
            .execute(
                "UPDATE skills SET name=?2, description=?3, instructions=?4, type=?5, updated_at=?6 WHERE id=?1",
                params![
                    id,
                    normalized.name,
                    normalized.description,
                    normalized.instructions,
                    normalized.skill_type,
                    now,
                ],
            )
            .map_err(|error| error.to_string())?;

        if changed == 0 {
            return Err(format!("No skill with id {id}"));
        }
        self.skill(id)?
            .ok_or_else(|| format!("Skill {id} vanished during the update"))
    }

    /// Turns a skill on or off without deleting it.
    ///
    /// Disabling rather than deleting is the default path because a skill is
    /// something a user spent effort writing and may want back unchanged.
    pub fn set_skill_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
        let changed = self
            .connection
            .lock()
            .map_err(|error| error.to_string())?
            .execute(
                "UPDATE skills SET enabled=?2, updated_at=?3 WHERE id=?1",
                params![id, enabled, crate::database::utc_now()],
            )
            .map_err(|error| error.to_string())?;
        if changed == 0 {
            return Err(format!("No skill with id {id}"));
        }
        Ok(())
    }

    /// Records that a reply used a skill.
    ///
    /// Increments rather than setting, so two runs in the same session both count.
    /// No row is touched when the id is unknown: this is called from the reply
    /// path, and failing a reply over a usage counter would be absurd.
    ///
    /// Called by [`crate::ai::runtime`] once the run has the resolved skill in
    /// hand, so the count reflects a skill that was really applied rather than one
    /// that was merely requested and resolved to nothing.
    pub fn record_skill_use(&self, id: &str) {
        let _ = self.connection.lock().ok().map(|connection| {
            connection.execute("UPDATE skills SET use_count=use_count+1 WHERE id=?1", [id])
        });
    }

    /// Deletes a skill and its usage history.
    pub fn delete_skill(&self, id: &str) -> Result<(), String> {
        let changed = self
            .connection
            .lock()
            .map_err(|error| error.to_string())?
            .execute("DELETE FROM skills WHERE id=?1", [id])
            .map_err(|error| error.to_string())?;
        if changed == 0 {
            return Err(format!("No skill with id {id}"));
        }
        Ok(())
    }
}

fn skill_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Skill> {
    Ok(Skill {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        instructions: row.get(3)?,
        skill_type: row.get(4)?,
        origin: SkillOrigin::from_str(&row.get::<_, String>(5)?),
        enabled: row.get::<_, i64>(6)? != 0,
        use_count: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(name: &str) -> NewSkill {
        NewSkill {
            name: name.into(),
            description: Some("a description".into()),
            instructions: "Prefer small functions.".into(),
            skill_type: Some("frontend".into()),
            origin: SkillOrigin::User,
        }
    }

    #[test]
    fn a_created_skill_comes_back_complete() {
        let database = Database::in_memory().expect("db");
        let created = database.create_skill(&skill("Style")).expect("create");

        assert!(!created.id.is_empty());
        assert_eq!(created.name, "Style");
        assert_eq!(created.instructions, "Prefer small functions.");
        assert_eq!(created.skill_type.as_deref(), Some("frontend"));
        assert!(created.enabled, "a new skill should be usable");
        assert_eq!(created.use_count, 0);
        assert_eq!(created.origin, SkillOrigin::User);
    }

    /// Whitespace a model produced around a field must never reach the picker.
    #[test]
    fn fields_are_trimmed_on_the_way_in() {
        let database = Database::in_memory().expect("db");
        let created = database
            .create_skill(&NewSkill {
                name: "  Style  ".into(),
                description: Some("  a description  ".into()),
                instructions: "  Prefer small functions.  ".into(),
                skill_type: Some("  frontend  ".into()),
                origin: SkillOrigin::User,
            })
            .expect("create");

        assert_eq!(created.name, "Style");
        assert_eq!(created.description.as_deref(), Some("a description"));
        assert_eq!(created.instructions, "Prefer small functions.");
        assert_eq!(created.skill_type.as_deref(), Some("frontend"));
    }

    /// An empty label and no label are different things when filtering, so a
    /// whitespace-only label becomes `None` rather than `Some("")`.
    #[test]
    fn a_blank_type_is_unlabelled_rather_than_empty() {
        let database = Database::in_memory().expect("db");
        let created = database
            .create_skill(&NewSkill {
                skill_type: Some("   ".into()),
                ..skill("Style")
            })
            .expect("create");

        assert_eq!(created.skill_type, None);
    }

    #[test]
    fn a_name_or_body_that_is_only_whitespace_is_refused() {
        let database = Database::in_memory().expect("db");
        assert!(database
            .create_skill(&NewSkill {
                name: "   ".into(),
                ..skill("Style")
            })
            .is_err());
        assert!(database
            .create_skill(&NewSkill {
                instructions: "\n\t ".into(),
                ..skill("Style")
            })
            .is_err());
    }

    #[test]
    fn an_over_long_field_is_refused_with_the_limit_named() {
        let database = Database::in_memory().expect("db");
        let error = database
            .create_skill(&NewSkill {
                name: "n".repeat(MAX_NAME_CHARS + 1),
                ..skill("Style")
            })
            .expect_err("too long");
        assert!(error.contains("name"), "{error}");
    }

    /// Usage has to accumulate: a skill used on three turns is used three times,
    /// which is the whole value of the counter.
    #[test]
    fn uses_accumulate_across_replies() {
        let database = Database::in_memory().expect("db");
        let created = database.create_skill(&skill("Style")).expect("create");

        for _ in 0..3 {
            database.record_skill_use(&created.id);
        }

        assert_eq!(
            database
                .skill(&created.id)
                .expect("read")
                .expect("row")
                .use_count,
            3
        );
    }

    /// A usage count must survive an edit. Otherwise editing a skill would look
    /// like a new one every time it was touched.
    #[test]
    fn editing_a_skill_keeps_its_id_history_and_origin() {
        let database = Database::in_memory().expect("db");
        let created = database.create_skill(&skill("Style")).expect("create");
        database.record_skill_use(&created.id);

        let updated = database
            .update_skill(
                &created.id,
                &NewSkill {
                    name: "House style".into(),
                    instructions: "Prefer pure functions.".into(),
                    origin: SkillOrigin::User,
                    ..skill("ignored")
                },
            )
            .expect("update");

        assert_eq!(updated.id, created.id);
        assert_eq!(updated.name, "House style");
        assert_eq!(updated.instructions, "Prefer pure functions.");
        assert_eq!(updated.use_count, 1);
        assert_eq!(updated.origin, SkillOrigin::User);
    }

    #[test]
    fn an_agent_drafted_skill_is_marked_as_generated() {
        let database = Database::in_memory().expect("db");
        let created = database
            .create_skill(&NewSkill {
                origin: SkillOrigin::Generated,
                ..skill("House style")
            })
            .expect("create");

        // The distinction has to survive the round trip, or the UI cannot tell a
        // skill the user wrote from one the model inferred.
        assert_eq!(created.origin, SkillOrigin::Generated);
        assert_eq!(
            database
                .skill(&created.id)
                .expect("read")
                .expect("row")
                .origin,
            SkillOrigin::Generated
        );
    }

    /// A disabled skill is kept, not deleted: the work of writing it is not
    /// undone by a switch.
    #[test]
    fn a_disabled_skill_is_kept_but_stops_being_usable() {
        let database = Database::in_memory().expect("db");
        let created = database.create_skill(&skill("Style")).expect("create");

        database
            .set_skill_enabled(&created.id, false)
            .expect("disable");

        assert_eq!(database.list_skills().expect("all").len(), 1);
        assert!(database.enabled_skills().expect("enabled").is_empty());
        assert!(
            !database
                .skill(&created.id)
                .expect("read")
                .expect("row")
                .enabled
        );
    }

    #[test]
    fn enabled_skills_come_back_by_name() {
        let database = Database::in_memory().expect("db");
        for name in ["Zebra", "apple", "Mango"] {
            database.create_skill(&skill(name)).expect("create");
        }

        let names = database
            .enabled_skills()
            .expect("enabled")
            .into_iter()
            .map(|skill| skill.name)
            .collect::<Vec<_>>();
        // Case-insensitive, so the list does not reorder when a name is renamed
        // from lower to upper case.
        assert_eq!(names, vec!["apple", "Mango", "Zebra"]);
    }

    #[test]
    fn deleting_removes_the_skill() {
        let database = Database::in_memory().expect("db");
        let created = database.create_skill(&skill("Style")).expect("create");

        database.delete_skill(&created.id).expect("delete");

        assert!(database.skill(&created.id).expect("read").is_none());
    }

    /// Every id-addressed write reports an unknown id rather than silently
    /// succeeding, which would leave the UI showing a change that never happened.
    #[test]
    fn writing_to_an_unknown_skill_is_reported() {
        let database = Database::in_memory().expect("db");
        assert!(database.update_skill("nope", &skill("Style")).is_err());
        assert!(database.set_skill_enabled("nope", true).is_err());
        assert!(database.delete_skill("nope").is_err());
    }

    /// Counting a use of a skill that was deleted mid-reply must not fail the
    /// reply. Silently ignored, because this runs on the reply path.
    #[test]
    fn counting_a_use_of_a_deleted_skill_is_silent() {
        let database = Database::in_memory().expect("db");
        database.record_skill_use("never-existed");
    }

    /// `type` is one shared vocabulary, so filtering by it has to group
    /// correctly across skills that use the same label.
    #[test]
    fn skills_can_be_grouped_by_their_label() {
        let database = Database::in_memory().expect("db");
        database.create_skill(&skill("A")).expect("create");
        database
            .create_skill(&NewSkill {
                skill_type: Some("rust".into()),
                ..skill("B")
            })
            .expect("create");
        database
            .create_skill(&NewSkill {
                skill_type: Some("rust".into()),
                ..skill("C")
            })
            .expect("create");

        let rust = database
            .list_skills()
            .expect("all")
            .into_iter()
            .filter(|skill| skill.skill_type.as_deref() == Some("rust"))
            .count();
        assert_eq!(rust, 2);
    }

    #[test]
    fn origin_round_trips_through_its_stored_spelling() {
        assert_eq!(SkillOrigin::from_str("generated"), SkillOrigin::Generated);
        assert_eq!(SkillOrigin::Generated.as_str(), "generated");
        assert_eq!(SkillOrigin::User.as_str(), "user");
        // An unreadable value reads as deliberate, which is the safer default.
        assert_eq!(SkillOrigin::from_str("something-else"), SkillOrigin::User);
    }

    #[test]
    fn the_shared_timestamp_is_the_shape_the_database_compares_as_text() {
        // Everything else in this database sorts timestamps as strings, so a
        // different shape here would place a skill wrongly in that ordering.
        let stamp = crate::database::utc_now();
        assert!(stamp.ends_with('Z'), "{stamp}");
        assert!(stamp.contains('T'), "{stamp}");
        // Two writes a moment apart must order correctly as plain text, which is
        // the whole reason the format is fixed.
        assert!(
            crate::database::utc_now() >= stamp,
            "the clock went backwards"
        );
    }
}
