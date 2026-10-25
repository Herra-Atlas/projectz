//! The installation-wide tool switches.
//!
//! A per-run access set answers "what may this job, message or conversation do".
//! This answers the outer question: "what may anything on this installation do".
//! It is intersected into every run before its approval gate is built, so a
//! terminal switched off here stays off whatever a job row, a Telegram access
//! record or a sub-agent asks for.
//!
//! Stored under its own settings key rather than a new table, like the MCP server
//! list and the messaging record: the shape is one small record and adding a
//! capability later must not cost a schema version.

use crate::ai::tools::AccessSet;
use crate::database::Database;

/// Where the host record lives.
pub const KEY: &str = "app.host_access";

/// The switches in force, defaulting to everything.
///
/// A missing record means the user never narrowed anything, which has to behave
/// exactly like every version before this one existed.
pub fn load(database: &Database) -> AccessSet {
    database.setting::<AccessSet>(KEY).unwrap_or_default()
}

/// Replaces the host record.
pub fn save(database: &Database, access: AccessSet) -> Result<AccessSet, String> {
    let value = serde_json::to_value(access)
        .map_err(|error| format!("Could not store the host access settings: {error}"))?;
    database.set_setting(KEY, &value)?;
    Ok(access)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_record_allows_everything() {
        let database = Database::open_in_memory();
        database.initialize().expect("migrate");
        assert_eq!(load(&database), AccessSet::ALL);
    }

    #[test]
    fn a_saved_record_narrows_every_future_run() {
        let database = Database::open_in_memory();
        database.initialize().expect("migrate");
        let wanted = AccessSet {
            terminal: false,
            mcp: false,
            ..AccessSet::ALL
        };
        save(&database, wanted).expect("save");
        assert_eq!(load(&database), wanted);
    }
}
