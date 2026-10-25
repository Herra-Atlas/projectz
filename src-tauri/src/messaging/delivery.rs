//! Where a scheduled job reports back.
//!
//! A job created from Telegram must reach the person who asked, not every id on
//! the allowlist and not only while they still have the conversation open. The
//! run that calls `schedule_job` carries its channel target on the `RunHandle`;
//! the tool records it here against the job id at creation time.
//!
//! A separate settings key rather than a column: the job row is written by the
//! scheduler and the Jobs page, and neither has any business carrying a Telegram
//! chat address. A target that outlives a deleted job is inert, and the map is
//! bounded by the number of jobs a user creates.

use std::collections::HashMap;

use crate::ai::types::DeliveryTarget;
use crate::database::Database;

pub const KEY: &str = "app.messaging.job_targets";

fn load_all(database: &Database) -> HashMap<String, DeliveryTarget> {
    database.setting(KEY).unwrap_or_default()
}

/// Records who asked for a job.
pub fn remember(database: &Database, job_id: &str, target: &DeliveryTarget) -> Result<(), String> {
    let mut all = load_all(database);
    all.insert(job_id.to_string(), target.clone());
    let value = serde_json::to_value(all)
        .map_err(|error| format!("Could not store the job's delivery target: {error}"))?;
    database.set_setting(KEY, &value)
}

/// The target for a job, if it was created from a channel.
pub fn target_for(database: &Database, job_id: &str) -> Option<DeliveryTarget> {
    load_all(database).remove(job_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> Database {
        let database = Database::open_in_memory();
        database.initialize().expect("migrate");
        database
    }

    #[test]
    fn a_jobs_target_round_trips_and_does_not_leak_to_another_job() {
        let database = database();
        let target = DeliveryTarget {
            channel: "telegram".into(),
            user_id: "42".into(),
            chat_id: "42".into(),
        };
        remember(&database, "job-1", &target).expect("remember");
        assert_eq!(target_for(&database, "job-1"), Some(target));
        assert!(target_for(&database, "job-2").is_none());
    }
}
