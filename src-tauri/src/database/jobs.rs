//! Scheduled jobs: saved instructions the app runs on its own.
//!
//! # What a job is
//!
//! A name, an instruction, the model to run it on, when it should run, and what
//! the machine has to look like first. The two halves are stored side by side but
//! mean different things: `schedule` is *when the user wants it*, and `conditions`
//! is *when it is safe to start* -- the second is why a job set for 3am may not
//! actually begin at 3am.
//!
//! # Why the values are JSON columns
//!
//! `model`, `schedule`, `conditions`, `access` and `depends_on` are shapes the
//! scheduler matches on, and each one is a small record rather than a scalar. A
//! column per field would make adding a condition a migration; a JSON column
//! keeps the table's shape fixed while the record it carries can grow. The cost
//! is that these cannot be indexed or queried in SQL, which is fine: the only
//! column the scheduler queries is `next_run_at`, and that one is real.
//!
//! # Why `job_runs` is separate
//!
//! A job runs many times and the job row holds only the *last* result, because
//! that is what a list needs to show. The pair (status, reason) per attempt is
//! history, and history is what someone opens when a job did not do what they
//! expected -- so each attempt is its own row, cascading with the job.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::ai::tools::{AccessSet, PermissionMode, ToolMode};
use crate::database::Database;

/// Which model a job runs on.
///
/// Tagged so the two shapes round-trip through one column. A local job names a
/// stored model id; the runner resolves it to the running engine the same way a
/// hand-started chat does, and fails loudly if the engine is not up rather than
/// silently using a different model.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum JobModel {
    /// A local model, by the id in `local_models`.
    Local { id: String },
    /// A provider endpoint and one of its models.
    Remote { endpoint_id: String, model: String },
}

impl Default for JobModel {
    /// A local model with no id, which the runner refuses to start.
    ///
    /// Reached only when a stored `model_json` cannot be parsed: it fails the next
    /// run with a visible reason rather than letting the row disappear.
    fn default() -> Self {
        Self::Local { id: String::new() }
    }
}

/// When a job is due.
///
/// A tagged union rather than a record of optional fields, because exactly one
/// shape is ever in force and the editor offers them as one choice. A record of
/// four options would allow states that mean nothing -- two set at once -- and the
/// scheduler would have to invent a precedence for them.
///
/// `weekday` is `0..=6`, Monday first, matching what the editor lists.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum JobSchedule {
    /// Run once. `None` means as soon as possible.
    Once { at: Option<String> },
    /// Repeat every N minutes.
    Every { minutes: u32 },
    /// Repeat once a day at a local time, `"HH:MM"`.
    Daily { at: String },
    /// Repeat weekly on a weekday, at a local time or as soon as that day starts.
    Weekly { weekday: u8, at: Option<String> },
}

impl Default for JobSchedule {
    /// A one-shot due immediately: the only shape that cannot repeat forever, so a
    /// schedule that cannot be read does the least surprising thing.
    fn default() -> Self {
        Self::Once { at: None }
    }
}

/// What the machine must look like before a job starts, and whether it may be
/// interrupted once it has.
///
/// Each limit is optional: `None` means "do not care", which is the honest
/// default for a machine we cannot read (a GPU with no probe reports nothing, and
/// a job must not be blocked forever by a figure that will never arrive).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct JobConditions {
    /// Do not start while RAM use is at or above this percentage.
    pub max_ram_percent: Option<f32>,
    /// Do not start while VRAM use is at or above this percentage.
    pub max_vram_percent: Option<f32>,
    /// Do not start until the machine has been idle at least this long.
    pub min_idle_seconds: Option<u64>,
    /// Whether a run already in flight is stopped when a limit is exceeded.
    ///
    /// The switch the user asked for: off means the job finishes whatever the PC
    /// is doing, on means it yields. Defaults to on -- a job should not fight the
    /// person using the machine -- and a job on a remote model can turn it off
    /// because it costs the PC nothing to run.
    pub stop_when_busy: bool,
}

impl Default for JobConditions {
    fn default() -> Self {
        Self {
            max_ram_percent: Some(60.0),
            max_vram_percent: Some(30.0),
            min_idle_seconds: None,
            stop_when_busy: true,
        }
    }
}

/// A stored job.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub name: String,
    pub prompt: String,
    /// The folder the run resolves against. `None` uses the open workspace.
    pub workspace: Option<String>,
    pub mode: ToolMode,
    pub permission: PermissionMode,
    pub access: AccessSet,
    pub model: JobModel,
    pub schedule: JobSchedule,
    pub conditions: JobConditions,
    /// Jobs in the same lane never run at the same time.
    pub lane: String,
    /// Jobs that must have succeeded before this one may start.
    pub depends_on: Vec<String>,
    pub enabled: bool,
    /// When the scheduler should next consider it, as UTC ISO-8601.
    pub next_run_at: Option<String>,
    pub last_run_at: Option<String>,
    pub last_status: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// What the editor sends to create or replace a job.
///
/// Separate from [`Job`] so a caller cannot set the scheduler's own fields
/// (`next_run_at`, `last_*`) by accident.
#[derive(Clone, Debug, Deserialize)]
pub struct NewJob {
    pub name: String,
    pub prompt: String,
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default = "agent")]
    pub mode: ToolMode,
    #[serde(default = "full")]
    pub permission: PermissionMode,
    #[serde(default)]
    pub access: AccessSet,
    pub model: JobModel,
    #[serde(default)]
    pub schedule: JobSchedule,
    #[serde(default)]
    pub conditions: JobConditions,
    #[serde(default = "default_lane")]
    pub lane: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default = "enabled")]
    pub enabled: bool,
}

fn agent() -> ToolMode {
    ToolMode::Agent
}
fn full() -> PermissionMode {
    PermissionMode::Full
}
fn default_lane() -> String {
    "default".to_string()
}
fn enabled() -> bool {
    true
}

/// One attempt at a job.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobRun {
    pub id: String,
    pub job_id: String,
    /// The session the attempt ran as, so its transcript can be opened.
    pub session_id: Option<String>,
    /// `queued`, `running`, `completed`, `failed`, `stopped` or `skipped`.
    pub status: String,
    /// Why it ended that way, in the user's words. `None` while it is fine.
    pub reason: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub created_at: String,
}

const JOB_COLUMNS: &str = "id,name,prompt,workspace,mode,permission,access_json,model_json,schedule_json,conditions_json,lane,depends_on_json,enabled,next_run_at,last_run_at,last_status,created_at,updated_at";

impl Database {
    /// Every job, by name. The editor's list.
    pub fn list_jobs(&self) -> Result<Vec<Job>, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {JOB_COLUMNS} FROM jobs ORDER BY name COLLATE NOCASE"
            ))
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], job_from_row)
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        Ok(rows)
    }

    /// One job, or `None` when it does not exist.
    pub fn job(&self, id: &str) -> Result<Option<Job>, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        connection
            .query_row(
                &format!("SELECT {JOB_COLUMNS} FROM jobs WHERE id=?1"),
                [id],
                job_from_row,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    /// The jobs the scheduler should consider now: enabled and due.
    pub fn due_jobs(&self, now: &str) -> Result<Vec<Job>, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {JOB_COLUMNS} FROM jobs
                 WHERE enabled=1 AND next_run_at IS NOT NULL AND next_run_at <= ?1
                 ORDER BY next_run_at"
            ))
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([now], job_from_row)
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        Ok(rows)
    }

    /// Creates a job, returning the stored row.
    ///
    /// `next_run_at` is supplied by the caller rather than stamped here, because
    /// what "first due" means depends on the schedule: an interval job is due now,
    /// a one-shot is due at its moment. That decision lives with the schedule
    /// (`jobs::schedule::first_run`) instead of being split across two layers.
    pub fn create_job(&self, job: &NewJob, next_run_at: Option<&str>) -> Result<Job, String> {
        let name = job.name.trim();
        if name.is_empty() {
            return Err("A job needs a name".to_string());
        }
        if job.prompt.trim().is_empty() {
            return Err("A job needs an instruction".to_string());
        }
        let now = crate::database::utc_now();
        let id = uuid::Uuid::new_v4().to_string();
        {
            let connection = self.connection.lock().map_err(|error| error.to_string())?;
            connection
                .execute(
                    "INSERT INTO jobs (id,name,prompt,workspace,mode,permission,access_json,model_json,schedule_json,conditions_json,lane,depends_on_json,enabled,next_run_at,created_at,updated_at)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?15)",
                    params![
                        id,
                        name,
                        job.prompt.trim(),
                        job.workspace,
                        wire(&job.mode)?,
                        wire(&job.permission)?,
                        json(&job.access)?,
                        json(&job.model)?,
                        json(&job.schedule)?,
                        json(&job.conditions)?,
                        job.lane.trim(),
                        json(&job.depends_on)?,
                        job.enabled as i64,
                        next_run_at,
                        now,
                    ],
                )
                .map_err(|error| error.to_string())?;
        }
        self.job(&id)?
            .ok_or_else(|| format!("Job {id} vanished immediately after being written"))
    }

    /// Replaces the editable fields, keeping the scheduler's own bookkeeping.
    pub fn update_job(&self, id: &str, job: &NewJob) -> Result<Job, String> {
        let name = job.name.trim();
        if name.is_empty() {
            return Err("A job needs a name".to_string());
        }
        if job.prompt.trim().is_empty() {
            return Err("A job needs an instruction".to_string());
        }
        let now = crate::database::utc_now();
        let changed = {
            let connection = self.connection.lock().map_err(|error| error.to_string())?;
            connection
                .execute(
                    "UPDATE jobs SET name=?2, prompt=?3, workspace=?4, mode=?5, permission=?6,
                        access_json=?7, model_json=?8, schedule_json=?9, conditions_json=?10,
                        lane=?11, depends_on_json=?12, enabled=?13, updated_at=?14
                     WHERE id=?1",
                    params![
                        id,
                        name,
                        job.prompt.trim(),
                        job.workspace,
                        wire(&job.mode)?,
                        wire(&job.permission)?,
                        json(&job.access)?,
                        json(&job.model)?,
                        json(&job.schedule)?,
                        json(&job.conditions)?,
                        job.lane.trim(),
                        json(&job.depends_on)?,
                        job.enabled as i64,
                        now,
                    ],
                )
                .map_err(|error| error.to_string())?
        };
        if changed == 0 {
            return Err(format!("No job with id {id}"));
        }
        self.job(id)?
            .ok_or_else(|| format!("Job {id} vanished immediately after being written"))
    }

    pub fn delete_job(&self, id: &str) -> Result<(), String> {
        let changed = self
            .connection
            .lock()
            .map_err(|error| error.to_string())?
            .execute("DELETE FROM jobs WHERE id=?1", [id])
            .map_err(|error| error.to_string())?;
        if changed == 0 {
            return Err(format!("No job with id {id}"));
        }
        Ok(())
    }

    /// The job behind a transcript, when that transcript is one of its runs.
    ///
    /// The other direction from [`Database::job_runs`]: given what the sidebar
    /// deleted, which job -- if any -- does it belong to.
    pub fn job_id_for_session(&self, session_id: &str) -> Result<Option<String>, String> {
        self.connection
            .lock()
            .map_err(|error| error.to_string())?
            .query_row(
                "SELECT job_id FROM job_runs WHERE session_id=?1",
                [session_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    /// Deletes the job a transcript belongs to, along with the rest of its runs.
    ///
    /// A job's transcripts are what it produced, so deleting one from the sidebar
    /// means the same thing as deleting the job: leaving the job behind would keep
    /// listing work the reader just got rid of, and leaving its other transcripts
    /// behind would keep them in the sidebar owned by nothing.
    ///
    /// Every transcript of the job goes, the one that named it included -- deleting a
    /// row that is already gone is a no-op, so this is the same call whether the
    /// caller removed it first or not.
    ///
    /// Returns the job that went, if there was one.
    pub fn delete_job_for_session(&self, session_id: &str) -> Result<Option<String>, String> {
        let Some(job_id) = self.job_id_for_session(session_id)? else {
            return Ok(None);
        };
        let transcripts: Vec<String> = self
            .job_runs(&job_id, i64::MAX)?
            .into_iter()
            .filter_map(|run| run.session_id)
            .collect();
        self.delete_job(&job_id)?;
        for id in transcripts {
            if let Err(error) = self.delete_chat_session(&id) {
                tracing::warn!(session = %id, %error, "a job transcript could not be removed");
            }
        }
        Ok(Some(job_id))
    }

    /// Turns a job on or off.
    ///
    /// Enabling also re-arms the clock, so a job switched on after a long pause is
    /// evaluated now rather than at a next-run time from before it was paused.
    pub fn set_job_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
        let now = crate::database::utc_now();
        let changed = self
            .connection
            .lock()
            .map_err(|error| error.to_string())?
            .execute(
                "UPDATE jobs SET enabled=?2, next_run_at=CASE WHEN ?2=1 THEN ?3 ELSE NULL END, updated_at=?3 WHERE id=?1",
                params![id, enabled as i64, now],
            )
            .map_err(|error| error.to_string())?;
        if changed == 0 {
            return Err(format!("No job with id {id}"));
        }
        Ok(())
    }

    /// Records what the last attempt was and when the next one is due.
    pub fn touch_job(
        &self,
        id: &str,
        last_status: &str,
        next_run_at: Option<&str>,
    ) -> Result<(), String> {
        let now = crate::database::utc_now();
        self.connection
            .lock()
            .map_err(|error| error.to_string())?
            .execute(
                "UPDATE jobs SET last_run_at=?2, last_status=?3, next_run_at=?4, updated_at=?2 WHERE id=?1",
                params![id, now, last_status, next_run_at],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// Opens a run row for an attempt that is about to start.
    pub fn start_job_run(&self, job_id: &str, session_id: Option<&str>) -> Result<String, String> {
        let now = crate::database::utc_now();
        let id = uuid::Uuid::new_v4().to_string();
        self.connection
            .lock()
            .map_err(|error| error.to_string())?
            .execute(
                "INSERT INTO job_runs (id,job_id,session_id,status,started_at,created_at)
                 VALUES (?1,?2,?3,'running',?4,?4)",
                params![id, job_id, session_id, now],
            )
            .map_err(|error| error.to_string())?;
        Ok(id)
    }

    /// Sets when a job is next due, leaving the last-run fields alone.
    ///
    /// Separate from [`Database::touch_job`] because a deferral is not a run:
    /// nothing happened, so only the time it will be looked at again changes. Using
    /// `touch_job` here would stamp `last_run_at` for a job that never started.
    pub fn set_job_next_run(&self, id: &str, next: Option<&str>) -> Result<(), String> {
        self.connection
            .lock()
            .map_err(|error| error.to_string())?
            .execute(
                "UPDATE jobs SET next_run_at=?2 WHERE id=?1",
                params![id, next],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// Closes a run row with its outcome and the session it ran as.
    ///
    /// The session is written here rather than at start because it is only known
    /// once the run has built one; `COALESCE` keeps a value already set by a caller
    /// that knew it up front.
    pub fn finish_job_run(
        &self,
        run_id: &str,
        status: &str,
        reason: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<(), String> {
        let now = crate::database::utc_now();
        self.connection
            .lock()
            .map_err(|error| error.to_string())?
            .execute(
                "UPDATE job_runs SET status=?2, reason=?3, finished_at=?4, session_id=COALESCE(?5, session_id) WHERE id=?1",
                params![run_id, status, reason, now, session_id],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// One job's attempts, newest first.
    pub fn job_runs(&self, job_id: &str, limit: i64) -> Result<Vec<JobRun>, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let mut statement = connection
            .prepare(
                "SELECT id,job_id,session_id,status,reason,started_at,finished_at,created_at
                 FROM job_runs WHERE job_id=?1 ORDER BY started_at DESC LIMIT ?2",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![job_id, limit], |row| {
                Ok(JobRun {
                    id: row.get(0)?,
                    job_id: row.get(1)?,
                    session_id: row.get(2)?,
                    status: row.get(3)?,
                    reason: row.get(4)?,
                    started_at: row.get(5)?,
                    finished_at: row.get(6)?,
                    created_at: row.get(7)?,
                })
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        Ok(rows)
    }

    /// When a job last finished successfully, for a `depends_on` check.
    pub fn last_success_at(&self, job_id: &str) -> Result<Option<String>, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        connection
            .query_row(
                "SELECT MAX(finished_at) FROM job_runs WHERE job_id=?1 AND status='completed'",
                [job_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .map_err(|error| error.to_string())
    }
}

/// One row as a [`Job`].
fn job_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Job> {
    Ok(Job {
        id: row.get(0)?,
        name: row.get(1)?,
        prompt: row.get(2)?,
        workspace: row.get(3)?,
        mode: from_wire(&row.get::<_, String>(4)?).map_err(decode_error)?,
        permission: from_wire(&row.get::<_, String>(5)?).map_err(decode_error)?,
        access: decode(&row.get::<_, String>(6)?),
        model: decode(&row.get::<_, String>(7)?),
        schedule: decode(&row.get::<_, String>(8)?),
        conditions: decode(&row.get::<_, String>(9)?),
        lane: row.get(10)?,
        depends_on: decode(&row.get::<_, String>(11)?),
        enabled: row.get::<_, i64>(12)? != 0,
        next_run_at: row.get(13)?,
        last_run_at: row.get(14)?,
        last_status: row.get(15)?,
        created_at: row.get(16)?,
        updated_at: row.get(17)?,
    })
}

/// A JSON column decoded, falling back to its default when unreadable.
///
/// Lenient on purpose: a row the app cannot parse should not take the whole list
/// with it. A job whose model decodes to nothing fails its next run with a visible
/// reason, which is better than a list that refuses to open.
fn decode<T: serde::de::DeserializeOwned + Default>(text: &str) -> T {
    serde_json::from_str(text).unwrap_or_default()
}

fn decode_error(error: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
    )
}

/// The plain wire form of a serde-tagged enum, for a TEXT column.
fn wire<T: Serialize>(value: &T) -> Result<String, String> {
    match serde_json::to_value(value).map_err(|error| error.to_string())? {
        serde_json::Value::String(text) => Ok(text),
        other => Ok(other.to_string()),
    }
}

fn from_wire<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, String> {
    serde_json::from_value(serde_json::Value::String(text.to_string()))
        .map_err(|error| error.to_string())
}

fn json<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(name: &str) -> NewJob {
        NewJob {
            name: name.to_string(),
            prompt: "Summarise the repo".to_string(),
            workspace: None,
            mode: ToolMode::Agent,
            permission: PermissionMode::Full,
            access: AccessSet {
                terminal: false,
                ..AccessSet::ALL
            },
            model: JobModel::Remote {
                endpoint_id: "ep".to_string(),
                model: "gpt".to_string(),
            },
            schedule: JobSchedule::Every { minutes: 30 },
            conditions: JobConditions::default(),
            lane: "default".to_string(),
            depends_on: Vec::new(),
            enabled: true,
        }
    }

    #[test]
    fn a_created_job_round_trips_every_field() {
        let database = Database::in_memory().expect("db");
        let created = database
            .create_job(&job("Nightly"), Some(&crate::database::utc_now()))
            .expect("create");
        let read = database.job(&created.id).expect("read").expect("exists");
        assert_eq!(read.name, "Nightly");
        assert_eq!(read.schedule, JobSchedule::Every { minutes: 30 });
        assert!(
            !read.access.terminal,
            "the access set must survive the round trip"
        );
        assert_eq!(
            read.model,
            JobModel::Remote {
                endpoint_id: "ep".to_string(),
                model: "gpt".to_string()
            }
        );
        // Stamped due immediately, so the next tick evaluates it.
        assert!(read.next_run_at.is_some());
    }

    #[test]
    fn a_job_without_a_name_or_instruction_is_refused() {
        let database = Database::in_memory().expect("db");
        let mut blank = job("");
        assert!(database.create_job(&blank, None).is_err());
        blank.name = "Fine".to_string();
        blank.prompt = "   ".to_string();
        assert!(database.create_job(&blank, None).is_err());
    }

    #[test]
    fn only_enabled_due_jobs_come_back() {
        let database = Database::in_memory().expect("db");
        let due = database
            .create_job(&job("Due"), Some(&crate::database::utc_now()))
            .expect("create");
        let now = crate::database::utc_now();
        let found = database.due_jobs(&now).expect("due");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, due.id);

        database.set_job_enabled(&due.id, false).expect("disable");
        assert!(database.due_jobs(&now).expect("due").is_empty());
    }

    #[test]
    fn disenabling_clears_the_next_run_and_enabling_rearms_it() {
        let database = Database::in_memory().expect("db");
        let created = database
            .create_job(&job("Toggle"), Some(&crate::database::utc_now()))
            .expect("create");
        database.set_job_enabled(&created.id, false).expect("off");
        assert!(database
            .job(&created.id)
            .expect("read")
            .expect("exists")
            .next_run_at
            .is_none());
        database.set_job_enabled(&created.id, true).expect("on");
        assert!(database
            .job(&created.id)
            .expect("read")
            .expect("exists")
            .next_run_at
            .is_some());
    }

    #[test]
    fn a_run_records_its_outcome_and_is_listed_newest_first() {
        let database = Database::in_memory().expect("db");
        let created = database
            .create_job(&job("Runs"), Some(&crate::database::utc_now()))
            .expect("create");
        let run = database
            .start_job_run(&created.id, Some("session-1"))
            .expect("start");
        database
            .finish_job_run(&run, "completed", None, Some("session-1"))
            .expect("finish");
        let runs = database.job_runs(&created.id, 10).expect("history");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, "completed");
        assert_eq!(runs[0].session_id.as_deref(), Some("session-1"));
        assert!(database
            .last_success_at(&created.id)
            .expect("success")
            .is_some());
    }

    /// History belongs to the job: deleting the job must not leave runs behind.
    #[test]
    fn deleting_a_job_takes_its_runs_with_it() {
        let database = Database::in_memory().expect("db");
        // The in-memory constructors do not enable foreign keys, and this test is
        // about the cascade, so it is turned on for the life of the connection.
        database
            .connection
            .lock()
            .expect("lock")
            .pragma_update(None, "foreign_keys", "ON")
            .expect("pragma");
        let created = database
            .create_job(&job("Doomed"), Some(&crate::database::utc_now()))
            .expect("create");
        let run = database.start_job_run(&created.id, None).expect("start");
        database
            .finish_job_run(&run, "failed", Some("boom"), None)
            .expect("finish");
        database.delete_job(&created.id).expect("delete");
        assert!(database
            .job_runs(&created.id, 10)
            .expect("history")
            .is_empty());
    }

    /// Deleting a job's transcript from the sidebar deletes the job, and the job's
    /// other transcripts with it, so nothing is left pointing at nothing.
    #[test]
    fn deleting_a_jobs_transcript_deletes_the_job_and_its_other_transcripts() {
        let database = Database::in_memory().expect("db");
        // The in-memory constructors do not enable foreign keys, and this test is
        // about the cascade, so it is turned on for the life of the connection.
        database
            .connection
            .lock()
            .expect("lock")
            .pragma_update(None, "foreign_keys", "ON")
            .expect("pragma");
        for session in ["one", "two"] {
            database
                .save_chat_session(&serde_json::json!({
                    "id": session,
                    "title": session,
                    "messages": [],
                }))
                .expect("save");
        }
        let created = database
            .create_job(&job("Tidy"), Some(&crate::database::utc_now()))
            .expect("create");
        for session in ["one", "two"] {
            let run = database.start_job_run(&created.id, None).expect("start");
            database
                .finish_job_run(&run, "completed", None, Some(session))
                .expect("finish");
        }

        assert_eq!(
            database
                .job_id_for_session("one")
                .expect("lookup")
                .as_deref(),
            Some(created.id.as_str()),
            "the transcript did not point back at its job"
        );
        assert_eq!(
            database
                .delete_job_for_session("one")
                .expect("delete")
                .as_deref(),
            Some(created.id.as_str())
        );

        assert!(
            database.job(&created.id).expect("read").is_none(),
            "the job outlived its transcript"
        );
        assert!(database
            .job_runs(&created.id, 10)
            .expect("history")
            .is_empty());
        assert!(
            database.list_session_headers().expect("list").is_empty(),
            "the job's other transcript outlived its job"
        );
        // A conversation that belongs to no job is not a job's to delete.
        assert_eq!(
            database
                .delete_job_for_session("unrelated")
                .expect("delete"),
            None
        );
    }
}
