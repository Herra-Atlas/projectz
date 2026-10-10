//! When a job is due.
//!
//! # Four shapes, and the two that stop
//!
//! [`JobSchedule`] carries them: once, every N minutes, every day at a time, and
//! weekly on a weekday. The first is a one-shot -- it has no next time once its
//! moment has passed, which is what makes the job stop -- and the other three
//! repeat forever.
//!
//! # Two questions, not one
//!
//! [`next_run`] answers *"after this attempt, when?"* and [`first_run`] answers
//! *"now that it is saved, when first?"*. They are different for the same reason a
//! job should not sit idle for a whole interval after being created: an interval
//! job is due the moment it is saved, while its later runs are an interval apart.
//! Keeping them apart is also what lets "once, as soon as possible" exist at all --
//! it is stamped to a real moment when saved, and has nothing after that.
//!
//! # Local time, stored as UTC
//!
//! A daily or weekly time is what the wall clock reads, so it is read in the
//! machine's zone -- a user who says "3am Monday" means their Monday. The result is
//! converted to UTC before it is stored, because every timestamp in the database is
//! UTC and the scheduler compares them as strings. Daylight saving is handled by
//! asking the platform: a time in the spring-forward gap yields no candidate and
//! the job runs at the next one; a time in the fall-back hour takes the earlier
//! instant, so it runs once.

use chrono::{
    DateTime, Datelike, Duration, Local, LocalResult, NaiveDateTime, NaiveTime, TimeZone, Utc,
    Weekday,
};

use crate::database::jobs::JobSchedule;

/// The calendar format a stored timestamp uses, matching `utc_now`.
const STAMP: chrono::SecondsFormat = chrono::SecondsFormat::Millis;

/// A `DateTime` as the database writes and compares timestamps.
pub fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(STAMP, true)
}

/// The next time a job is due after an attempt, or `None` when it has none.
///
/// `None` is what ends a one-shot and what leaves an interval of zero alone;
/// either way the scheduler has nothing to fire on.
pub fn next_run(schedule: &JobSchedule, now: DateTime<Utc>) -> Option<String> {
    match schedule {
        // Once its moment is behind us there is nothing left -- which is the case
        // as soon as the run it caused has started, so the job runs exactly once.
        JobSchedule::Once { at } => {
            let at = at.as_deref().and_then(parse_stamp)?;
            (at > now).then(|| stamp(at))
        }
        JobSchedule::Every { minutes } => {
            (*minutes > 0).then(|| stamp(now + Duration::minutes(i64::from(*minutes))))
        }
        JobSchedule::Daily { at } => next_daily(parse_time(at)?, now).map(stamp),
        JobSchedule::Weekly { weekday, at } => {
            next_weekly(*weekday, at.as_deref().and_then(parse_time), now).map(stamp)
        }
    }
}

/// The time a job should first be looked at, when it is saved or switched on.
pub fn first_run(schedule: &JobSchedule, now: DateTime<Utc>) -> Option<String> {
    match schedule {
        // "As soon as possible": due now, and stamped as a real moment so there is
        // nothing left of it once it has run.
        JobSchedule::Once { at: None } => Some(stamp(now)),
        JobSchedule::Once { at: Some(at) } => match parse_stamp(at) {
            Some(at) if at > now => Some(stamp(at)),
            // A moment already behind us still runs once, rather than being
            // skipped because the app was shut when it arrived.
            Some(_) => Some(stamp(now)),
            None => None,
        },
        JobSchedule::Every { minutes } => (*minutes > 0).then(|| stamp(now)),
        JobSchedule::Daily { at } => next_daily(parse_time(at)?, now).map(stamp),
        JobSchedule::Weekly { weekday, at } => {
            // A weekday with no time means "as soon as possible that day", so a job
            // saved on its own day runs now instead of waiting a week. Read from
            // `now` rather than the wall clock so the rule is the same one the rest
            // of this function obeys, and so it can be tested.
            let today = at.is_none()
                && weekday_from_index(*weekday)
                    .is_some_and(|day| day == now.with_timezone(&Local).weekday());
            if today {
                return Some(stamp(now));
            }
            next_weekly(*weekday, at.as_deref().and_then(parse_time), now).map(stamp)
        }
    }
}

/// The next occurrence of a wall-clock time, today or tomorrow.
fn next_daily(at: NaiveTime, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let local = now.with_timezone(&Local);
    if let Some(candidate) = resolve(local.date_naive().and_time(at)) {
        if candidate > now {
            return Some(candidate);
        }
    }
    resolve(local.date_naive().succ_opt()?.and_time(at))
}

/// The next occurrence of a weekday, at its time or at the start of the day.
///
/// Strictly in the future, which is what keeps a weekly job from firing again and
/// again on the day it already ran: with no time set the candidate is that day's
/// midnight, and midnight is behind us for the whole of the day itself.
fn next_weekly(weekday: u8, at: Option<NaiveTime>, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let target = weekday_from_index(weekday)?;
    let local = now.with_timezone(&Local);
    for extra in 0..=7 {
        let date = local.date_naive() + Duration::days(extra);
        if date.weekday() != target {
            continue;
        }
        let naive = date.and_time(at.unwrap_or(NaiveTime::MIN));
        if let Some(candidate) = resolve(naive) {
            if candidate > now {
                return Some(candidate);
            }
        }
    }
    None
}

/// A local wall-clock time as UTC, coping with the DST edge cases.
fn resolve(naive: NaiveDateTime) -> Option<DateTime<Utc>> {
    match Local.from_local_datetime(&naive) {
        LocalResult::Single(at) => Some(at.with_timezone(&Utc)),
        LocalResult::Ambiguous(earlier, _) => Some(earlier.with_timezone(&Utc)),
        LocalResult::None => None,
    }
}

/// `0..=6` as a weekday, Monday first, matching what the editor offers.
fn weekday_from_index(index: u8) -> Option<Weekday> {
    Some(match index {
        0 => Weekday::Mon,
        1 => Weekday::Tue,
        2 => Weekday::Wed,
        3 => Weekday::Thu,
        4 => Weekday::Fri,
        5 => Weekday::Sat,
        6 => Weekday::Sun,
        _ => return None,
    })
}

/// `"HH:MM"` as a time, ignoring anything else.
fn parse_time(text: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(text.trim(), "%H:%M").ok()
}

/// A stored UTC moment.
fn parse_stamp(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .expect("parse")
            .with_timezone(&Utc)
    }

    #[test]
    fn an_interval_is_measured_from_now() {
        let next = next_run(
            &JobSchedule::Every { minutes: 30 },
            at("2026-01-01T10:00:00Z"),
        );
        assert_eq!(next.as_deref(), Some("2026-01-01T10:30:00.000Z"));
    }

    /// An interval job is due the moment it is saved, not one interval later.
    #[test]
    fn an_interval_job_first_runs_now() {
        let first = first_run(
            &JobSchedule::Every { minutes: 30 },
            at("2026-01-01T10:00:00Z"),
        );
        assert_eq!(first.as_deref(), Some("2026-01-01T10:00:00.000Z"));
    }

    /// A zero interval would be due every tick, so it is no schedule at all.
    #[test]
    fn a_zero_interval_is_not_a_schedule() {
        let schedule = JobSchedule::Every { minutes: 0 };
        assert_eq!(next_run(&schedule, at("2026-01-01T10:00:00Z")), None);
        assert_eq!(first_run(&schedule, at("2026-01-01T10:00:00Z")), None);
    }

    /// A one-shot is due at its moment and never again -- the whole feature.
    #[test]
    fn a_one_shot_runs_once_and_then_has_no_next_time() {
        let schedule = JobSchedule::Once {
            at: Some("2026-01-02T03:00:00.000Z".to_string()),
        };
        assert_eq!(
            next_run(&schedule, at("2026-01-01T00:00:00Z")).as_deref(),
            Some("2026-01-02T03:00:00.000Z")
        );
        assert_eq!(next_run(&schedule, at("2026-01-02T03:00:01Z")), None);
    }

    /// "As soon as possible": due now when saved, and nothing after it has run.
    #[test]
    fn a_one_shot_with_no_time_is_due_immediately_and_only_once() {
        let schedule = JobSchedule::Once { at: None };
        assert_eq!(
            first_run(&schedule, at("2026-01-01T00:00:00Z")).as_deref(),
            Some("2026-01-01T00:00:00.000Z")
        );
        assert_eq!(next_run(&schedule, at("2026-01-01T00:00:00Z")), None);
    }

    /// A one-shot whose moment passed while the app was shut still runs once.
    #[test]
    fn a_missed_one_shot_runs_once_rather_than_being_skipped() {
        let schedule = JobSchedule::Once {
            at: Some("2025-12-31T03:00:00.000Z".to_string()),
        };
        assert_eq!(
            first_run(&schedule, at("2026-01-01T00:00:00Z")).as_deref(),
            Some("2026-01-01T00:00:00.000Z")
        );
    }

    /// A daily time is read in the machine's zone: whatever the offset, the result
    /// is exactly one day after the previous occurrence.
    #[test]
    fn a_daily_time_advances_by_one_day() {
        let schedule = JobSchedule::Daily {
            at: "03:00".to_string(),
        };
        let first = next_run(&schedule, at("2026-01-01T00:00:00Z")).expect("first");
        let second = next_run(&schedule, at(&first)).expect("second");
        let gap = DateTime::parse_from_rfc3339(&second)
            .expect("parse")
            .signed_duration_since(DateTime::parse_from_rfc3339(&first).expect("parse"));
        assert_eq!(gap, Duration::days(1), "{first} -> {second}");
    }

    /// A weekly job lands on the named weekday, and a second run is a week later.
    #[test]
    fn a_weekly_time_lands_on_its_weekday_and_repeats_weekly() {
        // 2026-01-01 is a Thursday.
        let schedule = JobSchedule::Weekly {
            weekday: 0,
            at: Some("09:00".to_string()),
        };
        let first = next_run(&schedule, at("2026-01-01T00:00:00Z")).expect("first");
        let landed = DateTime::parse_from_rfc3339(&first).expect("parse");
        assert_eq!(landed.with_timezone(&Local).weekday(), Weekday::Mon);
        let second = next_run(&schedule, landed.with_timezone(&Utc)).expect("second");
        let gap = DateTime::parse_from_rfc3339(&second)
            .expect("parse")
            .signed_duration_since(landed);
        assert_eq!(gap, Duration::days(7), "{first} -> {second}");
    }

    /// A weekday with no time is "as soon as possible that day". Saved on the day
    /// itself, that is now; afterwards it is the start of the next one.
    #[test]
    fn a_weekly_day_with_no_time_runs_as_soon_as_the_day_arrives() {
        let schedule = JobSchedule::Weekly {
            weekday: 3,
            at: None,
        };
        // Thursday 2026-01-01, saved that same day.
        assert_eq!(
            first_run(&schedule, at("2026-01-01T12:00:00Z")).as_deref(),
            Some("2026-01-01T12:00:00.000Z")
        );
        // After it has run, the next time is the start of the next Thursday.
        let next = next_run(&schedule, at("2026-01-01T12:00:00Z")).expect("next");
        let landed = DateTime::parse_from_rfc3339(&next).expect("parse");
        assert_eq!(landed.with_timezone(&Local).weekday(), Weekday::Thu);
        assert!(
            landed > at("2026-01-01T12:00:00Z"),
            "{next} was not in the future"
        );
    }

    #[test]
    fn a_day_that_is_not_a_weekday_is_refused() {
        let schedule = JobSchedule::Weekly {
            weekday: 9,
            at: None,
        };
        assert_eq!(next_run(&schedule, at("2026-01-01T00:00:00Z")), None);
    }

    #[test]
    fn a_malformed_time_is_ignored_rather_than_guessed_at() {
        assert_eq!(
            next_run(
                &JobSchedule::Daily {
                    at: "25:99".to_string()
                },
                at("2026-01-01T00:00:00Z")
            ),
            None
        );
    }
}
