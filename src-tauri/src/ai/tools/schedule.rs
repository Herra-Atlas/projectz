//! Scheduling work for later.
//!
//! # What this is
//!
//! One tool that writes a row into the same `jobs` table the Jobs screen writes to.
//! Nothing about *running* a job lives here: the scheduler picks the row up on its
//! next tick, in the same lane as any other job, and the run it starts is an ordinary
//! run. A tool that took its own route to execution would be a second scheduler to
//! keep correct, and the two would drift the first time either changed.
//!
//! # Why the job gets nothing the run did not have
//!
//! A job outlives the conversation that asked for it and runs with nobody watching,
//! so it is the one place in the app where a small permission now becomes a large one
//! later. Two rules close that:
//!
//! - **The tool has no `access` argument at all.** There is nothing to ask for, so
//!   nothing can be widened: the job is given the caller's access set exactly, and a
//!   read-only conversation can only ever schedule a read-only job. An argument that
//!   is intersected is a rule that has to be got right every time; an argument that
//!   does not exist cannot be got wrong.
//! - **`jobs` is the one capability a job's run is never given**, so scheduled work
//!   cannot schedule more of itself. One generation, by construction rather than by
//!   a depth counter somebody has to remember to check.
//!
//! # Why the schema takes no lane and no dependency
//!
//! Both would need reasoning the model cannot do. A lane is what serialises jobs, and
//! a schedule that runs alongside someone's own work is a schedule that fights it, so
//! every scheduled job lands in one lane of its own and the user can move it if they
//! want to. `depends_on` is a one-way promise: two jobs that wait for each other wait
//! forever, silently, and a model creating jobs one at a time cannot see the cycle it
//! is closing.

use std::sync::atomic::Ordering;

use chrono::{DateTime, Duration, NaiveTime, Utc};
use serde_json::Value;

use super::{workspace, Effect, ToolContext, ToolSpec};
use crate::ai::tools::{AccessSet, PermissionMode, ToolMode};
use crate::database::jobs::{JobConditions, JobModel, JobSchedule, NewJob};

/// How many jobs one run may create.
///
/// Not a security boundary -- the access set is -- but a confused model should not be
/// able to fill the Jobs screen, and a run that has said "and tomorrow, and tomorrow"
/// three times is confused.
const MAX_PER_RUN: usize = 2;

/// The shortest gap a repeating job may ask for.
///
/// A model asked to "keep an eye on this" will happily write `every 1 minutes`, which
/// is a token fire that nobody asked for and that reads as a runaway app.
const MIN_INTERVAL_MINUTES: u32 = 5;

/// The lane every scheduled job lands in.
const LANE: &str = "ai";

/// Saves a task to be run later, on the machine's own time.
pub const SCHEDULE_JOB: ToolSpec = ToolSpec {
    name: "schedule_job",
    description:
        "Save a task to be run later, on its own, without you. Use it when the user asks for \
something to happen at a time or on a schedule ('tonight', 'every morning', 'check it in an \
hour'), or when work should happen when the machine is free rather than now. The job runs in \
this workspace with the tools you have, but it does not see this conversation and cannot ask \
questions, so write the instruction as a complete brief. It waits its turn behind other \
scheduled jobs rather than running twice at once. You do not wait for it: say what you \
scheduled and carry on. Do not use it for work you can finish now -- for that, do the work.",
    parameters: r#"{
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "description": "A short name for the job, two to five words. It becomes the title of the job's transcript."
            },
            "instruction": {
                "type": "string",
                "description": "What the job should do when it runs, written as a complete brief with no reference to this conversation."
            },
            "run": {
                "type": "object",
                "description": "When it runs.",
                "properties": {
                    "kind": {
                        "type": "string",
                        "enum": ["once", "every", "daily", "weekly"],
                        "description": "`once` for a single run, `every` for a fixed gap between runs, `daily` for a time each day, `weekly` for a day of the week."
                    },
                    "in_minutes": {
                        "type": "integer",
                        "description": "For `once`: how many minutes from now to run. Omit to run as soon as it can."
                    },
                    "minutes": {
                        "type": "integer",
                        "description": "For `every`: the gap between runs in minutes, at least 5."
                    },
                    "at": {
                        "type": "string",
                        "description": "For `daily` and `weekly`: the wall-clock time to run, as HH:MM on a 24-hour clock. For `weekly` it may be omitted, meaning as soon as that day starts."
                    },
                    "weekday": {
                        "type": "integer",
                        "description": "For `weekly`: the day, 0 for Monday through 6 for Sunday."
                    }
                },
                "required": ["kind"],
                "additionalProperties": false
            }
        },
        "required": ["name", "instruction", "run"],
        "additionalProperties": false
    }"#,
    // A write because it changes stored state that outlives the run: its result is
    // never cacheable, and a run may not be handed a job created by an earlier turn.
    effect: Effect::Write,
    // No shell command line to quote in an approval prompt.
    command_argument: None,
    execute: |arguments, context| Box::pin(create(arguments, context)),
};

/// Creates the job, or explains why it will not.
async fn create(arguments: Value, context: ToolContext) -> Result<String, String> {
    let database = context
        .database
        .clone()
        .ok_or_else(|| "Scheduling needs a saved conversation".to_string())?;
    let run = context
        .run
        .clone()
        .ok_or_else(|| "Scheduling needs a run to belong to".to_string())?;

    let name = text(&arguments, "name").ok_or_else(|| "`name` is required".to_string())?;
    let instruction =
        text(&arguments, "instruction").ok_or_else(|| "`instruction` is required".to_string())?;

    if run.jobs_created.load(Ordering::Relaxed) >= MAX_PER_RUN {
        return Err(format!(
            "This run has already scheduled {MAX_PER_RUN} jobs, which is as many as one reply may \
             add. Do the rest of the work now, or tell the user what else deserves a job."
        ));
    }

    let now = Utc::now();
    let schedule = parse_schedule(arguments.get("run"), now)?;
    // Counted only once the arguments are known to be good, so a refused call does not
    // spend the run's budget.
    run.jobs_created.fetch_add(1, Ordering::Relaxed);

    let model = job_model(
        run.local,
        run.endpoint.id.as_str(),
        run.model.as_str(),
        || database.setting("local.selected_model"),
    )?;
    let job = database
        .create_job(
            &NewJob {
                name: name.clone(),
                prompt: instruction,
                // The folder this conversation is working in, so the job reads the same
                // project rather than whatever happens to be open when it fires.
                workspace: non_empty(workspace::root_string()),
                mode: ToolMode::Agent,
                // Unattended: there is nobody to answer a prompt, so a job that asks is a
                // job that parks forever. What actually constrains it is the access set.
                permission: PermissionMode::Full,
                // Exactly what this run holds, minus the ability to schedule more.
                // Read off the same gate the run's calls are checked against, so the
                // job cannot be given more than the run it came from.
                access: AccessSet {
                    jobs: false,
                    ..run.approval.access()
                },
                model,
                schedule: schedule.clone(),
                conditions: conditions_for(run.local),
                lane: LANE.to_string(),
                depends_on: Vec::new(),
                enabled: true,
            },
            crate::jobs::first_run(&schedule, now).as_deref(),
        )
        .map_err(|error| error.to_string())?;

    let when = match next_phrase(&job.next_run_at) {
        Some(at) => format!("first run {at}"),
        None => "no next run".to_string(),
    };
    Ok(format!(
        "Scheduled `{}`: {}. It runs in lane `{LANE}` with the tools this run has, and waits for \
         the job before it in that lane to finish. {when}.",
        job.name,
        describe(&schedule),
    ))
}

/// Reads a non-empty string argument.
fn text(arguments: &Value, key: &str) -> Option<String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// Turns a blank path into `None`, so an unopened workspace is not stored as `""`.
fn non_empty(value: String) -> Option<String> {
    let trimmed = value.trim().to_string();
    (!trimmed.is_empty()).then_some(trimmed)
}

/// The schedule the model asked for, or why it cannot be used.
///
/// Takes `now` rather than reading the clock so the one relative shape -- "in twenty
/// minutes" -- is turned into the absolute moment the database stores, and can be
/// tested without waiting twenty minutes.
fn parse_schedule(value: Option<&Value>, now: DateTime<Utc>) -> Result<JobSchedule, String> {
    let run = value.ok_or_else(|| "`run` is required".to_string())?;
    let kind = run.get("kind").and_then(Value::as_str).unwrap_or("once");
    match kind {
        "once" => match run.get("in_minutes").and_then(Value::as_i64) {
            Some(minutes) if minutes > 0 => Ok(JobSchedule::Once {
                at: Some(crate::jobs::stamp(now + Duration::minutes(minutes))),
            }),
            Some(_) => Err("`in_minutes` must be at least 1".to_string()),
            // As soon as it can, which is the one-shot the tool defaults to.
            None => Ok(JobSchedule::Once { at: None }),
        },
        "every" => {
            let minutes = run
                .get("minutes")
                .and_then(Value::as_u64)
                .ok_or_else(|| "`minutes` is required for an `every` job".to_string())?;
            if minutes < u64::from(MIN_INTERVAL_MINUTES) {
                return Err(format!(
                    "a repeating job cannot run more often than every {MIN_INTERVAL_MINUTES} \
                     minutes, and this one asked for {minutes}"
                ));
            }
            Ok(JobSchedule::Every {
                minutes: minutes.min(u64::from(u32::MAX)) as u32,
            })
        }
        "daily" => Ok(JobSchedule::Daily { at: time_at(run)? }),
        "weekly" => {
            let weekday = run
                .get("weekday")
                .and_then(Value::as_u64)
                .ok_or_else(|| "`weekday` is required for a `weekly` job".to_string())?;
            if weekday > 6 {
                return Err("`weekday` runs 0 for Monday through 6 for Sunday".to_string());
            }
            // A day with no time is the shape the editor also offers: as soon as that
            // day arrives.
            let at = match run.get("at").and_then(Value::as_str) {
                Some(text) => {
                    check_time(text)?;
                    Some(text.trim().to_string())
                }
                None => None,
            };
            Ok(JobSchedule::Weekly {
                weekday: weekday as u8,
                at,
            })
        }
        other => Err(format!(
            "`kind` must be once, every, daily or weekly, not `{other}`"
        )),
    }
}

/// The `at` a daily job needs.
fn time_at(run: &Value) -> Result<String, String> {
    let text = run
        .get("at")
        .and_then(Value::as_str)
        .ok_or_else(|| "`at` is required, as a time like 03:00".to_string())?;
    check_time(text)?;
    Ok(text.trim().to_string())
}

/// Whether a string is a wall-clock time, checked here so a typo is a tool error the
/// model can correct rather than a job that silently never runs.
fn check_time(text: &str) -> Result<(), String> {
    NaiveTime::parse_from_str(text.trim(), "%H:%M")
        .map(|_| ())
        .map_err(|_| format!("`{text}` is not a time -- use HH:MM on a 24-hour clock"))
}

/// The model the job should run on.
///
/// A job inherits the run's own model so it behaves the way the user expects the thing
/// they are watching, rather than swapping to a configured default behind them. A local
/// run is stored by the *model's* id rather than the server alias, because the job may
/// run hours later and has to name something the model list still knows.
fn job_model(
    local: bool,
    endpoint_id: &str,
    model: &str,
    selected_local: impl FnOnce() -> Option<String>,
) -> Result<JobModel, String> {
    if local {
        let id = selected_local().filter(|id| !id.trim().is_empty()).ok_or_else(|| {
            "This run is on a local model that is not recorded, so a job could not load it. \
             Pick the model in Settings > Local and try again."
                .to_string()
        })?;
        return Ok(JobModel::Local { id });
    }
    Ok(JobModel::Remote {
        endpoint_id: endpoint_id.to_string(),
        model: model.to_string(),
    })
}

/// What the machine has to look like for a job on this model.
///
/// A local model loads into memory and runs on the GPU, so its job waits for a machine
/// that is not already busy and yields if the user starts using it. A provider model
/// costs this machine nothing to run, so there is nothing to wait for and no reason to
/// stop halfway.
fn conditions_for(local: bool) -> JobConditions {
    if local {
        JobConditions {
            max_ram_percent: Some(60.0),
            max_vram_percent: Some(30.0),
            min_idle_seconds: Some(120),
            stop_when_busy: true,
        }
    } else {
        JobConditions {
            max_ram_percent: None,
            max_vram_percent: None,
            min_idle_seconds: None,
            stop_when_busy: false,
        }
    }
}

/// A schedule in the few words a sentence can carry.
fn describe(schedule: &JobSchedule) -> String {
    match schedule {
        JobSchedule::Once { at: None } => "once, as soon as it can".to_string(),
        JobSchedule::Once { at: Some(_) } => "once, at the time asked for".to_string(),
        JobSchedule::Every { minutes } => format!("every {minutes} minutes"),
        JobSchedule::Daily { at } => format!("every day at {at}"),
        JobSchedule::Weekly { weekday, at } => {
            const DAYS: [&str; 7] = [
                "Monday",
                "Tuesday",
                "Wednesday",
                "Thursday",
                "Friday",
                "Saturday",
                "Sunday",
            ];
            let day = DAYS.get(*weekday as usize).copied().unwrap_or("that day");
            match at {
                Some(at) => format!("every {day} at {at}"),
                None => format!("every {day}, as soon as the day starts"),
            }
        }
    }
}

/// When a job will next run, said the way a person would.
fn next_phrase(next: &Option<String>) -> Option<String> {
    let at = next.as_deref().and_then(|text| {
        DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|at| at.with_timezone(&chrono::Local))
    })?;
    Some(format!("next {}", at.format("%-d %B %H:%M")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .expect("parse")
            .with_timezone(&Utc)
    }

    #[test]
    fn the_schema_is_valid_json_and_names_its_required_fields() {
        let schema: Value =
            serde_json::from_str(SCHEDULE_JOB.parameters).expect("valid schema literal");
        assert_eq!(schema["required"][0], "name");
        assert!(schema["properties"]["run"].is_object());
        // No `access` and no `lane`: what cannot be asked for cannot be widened.
        assert!(schema["properties"]["access"].is_null());
        assert!(schema["properties"]["lane"].is_null());
        assert_eq!(schema["additionalProperties"], false);
    }

    /// The default shape: one run, as soon as it can.
    #[test]
    fn a_once_job_with_no_time_runs_as_soon_as_it_can() {
        let schedule = parse_schedule(Some(&json!({"kind": "once"})), at("2026-01-01T10:00:00Z"))
            .expect("once");
        assert_eq!(schedule, JobSchedule::Once { at: None });
    }

    /// A relative time is turned into the absolute moment the database stores.
    #[test]
    fn a_once_job_in_twenty_minutes_becomes_a_moment() {
        let schedule =
            parse_schedule(Some(&json!({"kind": "once", "in_minutes": 20})), at("2026-01-01T10:00:00Z"))
                .expect("once");
        assert_eq!(
            schedule,
            JobSchedule::Once {
                at: Some("2026-01-01T10:20:00.000Z".to_string())
            }
        );
    }

    /// A model asked to watch something will ask for a minute; a runaway loop is
    /// nobody's intent, so the floor refuses it.
    #[test]
    fn a_repeating_job_faster_than_the_floor_is_refused() {
        let error = parse_schedule(
            Some(&json!({"kind": "every", "minutes": 1})),
            at("2026-01-01T10:00:00Z"),
        )
        .expect_err("too fast");
        assert!(error.contains("every 5 minutes"), "{error}");
        assert!(parse_schedule(
            Some(&json!({"kind": "every", "minutes": 5})),
            at("2026-01-01T10:00:00Z")
        )
        .is_ok());
    }

    #[test]
    fn a_weekly_job_may_carry_a_time_or_not() {
        let timed = parse_schedule(
            Some(&json!({"kind": "weekly", "weekday": 4, "at": "17:00"})),
            at("2026-01-01T10:00:00Z"),
        )
        .expect("weekly");
        assert_eq!(
            timed,
            JobSchedule::Weekly {
                weekday: 4,
                at: Some("17:00".to_string())
            }
        );
        let untimed = parse_schedule(
            Some(&json!({"kind": "weekly", "weekday": 4})),
            at("2026-01-01T10:00:00Z"),
        )
        .expect("weekly");
        assert_eq!(untimed, JobSchedule::Weekly { weekday: 4, at: None });
    }

    /// A typo has to come back as something the model can correct, not as a job that
    /// sits in the list and never fires.
    #[test]
    fn a_malformed_time_is_refused_with_the_shape_to_use() {
        let error = parse_schedule(
            Some(&json!({"kind": "daily", "at": "5pm"})),
            at("2026-01-01T10:00:00Z"),
        )
        .expect_err("not a time");
        assert!(error.contains("HH:MM"), "{error}");
        assert!(parse_schedule(Some(&json!({"kind": "daily"})), at("2026-01-01T10:00:00Z")).is_err());
        assert!(parse_schedule(
            Some(&json!({"kind": "weekly", "weekday": 9})),
            at("2026-01-01T10:00:00Z")
        )
        .is_err());
        assert!(parse_schedule(Some(&json!({"kind": "hourly"})), at("2026-01-01T10:00:00Z")).is_err());
    }

    /// A local job waits for a machine that is free and yields if it stops being free.
    /// A provider job has nothing to wait for.
    #[test]
    fn a_local_job_waits_for_the_machine_and_a_provider_job_does_not() {
        let local = conditions_for(true);
        assert_eq!(local.max_ram_percent, Some(60.0));
        assert!(local.stop_when_busy);
        assert!(local.min_idle_seconds.is_some());

        let remote = conditions_for(false);
        assert_eq!(remote.max_ram_percent, None);
        assert!(!remote.stop_when_busy);
    }

    /// A local run is stored by the model's own id, because the job outlives the
    /// server process and has to name something the model list still knows.
    #[test]
    fn a_local_run_stores_the_models_id_rather_than_the_server_alias() {
        let model = job_model(true, "local", "server-alias", || {
            Some("qwen3-8b".to_string())
        })
        .expect("resolved");
        assert_eq!(
            model,
            JobModel::Local {
                id: "qwen3-8b".to_string()
            }
        );
        let error = job_model(true, "local", "server-alias", || None).expect_err("no model");
        assert!(error.contains("Settings > Local"), "{error}");
    }

    #[test]
    fn a_provider_run_keeps_its_endpoint_and_model() {
        let model = job_model(false, "openrouter", "gpt-5", || None).expect("resolved");
        assert_eq!(
            model,
            JobModel::Remote {
                endpoint_id: "openrouter".to_string(),
                model: "gpt-5".to_string()
            }
        );
    }

    #[test]
    fn a_schedule_reads_back_as_a_phrase() {
        assert_eq!(describe(&JobSchedule::Once { at: None }), "once, as soon as it can");
        assert_eq!(describe(&JobSchedule::Every { minutes: 30 }), "every 30 minutes");
        assert_eq!(
            describe(&JobSchedule::Daily { at: "03:00".to_string() }),
            "every day at 03:00"
        );
        assert_eq!(
            describe(&JobSchedule::Weekly {
                weekday: 0,
                at: Some("17:00".to_string())
            }),
            "every Monday at 17:00"
        );
    }
}
