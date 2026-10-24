//! Downloads in flight, and the ones that finished this session.
//!
//! # Why the list is in memory
//!
//! It is a view of work the app is doing, not an archive. The files are the
//! record; keeping a second index on disk would mean a list that outlives the
//! bytes it describes the moment someone deletes a model by hand.
//!
//! # Why pause is a flag rather than a cancelled task
//!
//! Cancelling a task mid-stream loses the connection and the buffers with it. The
//! stream loop reads this flag between chunks, flushes what it has and returns
//! `Paused` -- so the `.part` file on disk is exactly the resume point, and
//! resuming is a `Range` request from its length.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use serde::Serialize;

use super::kinds::Kind;

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum JobState {
    Running,
    Paused,
    Done,
    Stopped,
    Failed,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobView {
    /// `<kind>/<repo>/<path>`, which is also how a row finds its own job.
    pub id: String,
    pub kind: Kind,
    pub repo: String,
    pub path: String,
    pub name: String,
    /// Where the finished file lands, so a completed model can be registered
    /// without the frontend re-deriving the path.
    pub destination: String,
    pub state: JobState,
    pub received: u64,
    pub total: Option<u64>,
    /// Bytes per second, smoothed. Zero when nothing is moving.
    pub speed: u64,
    pub error: Option<String>,
}

/// What a running stream checks between chunks.
pub struct Flags {
    pause: AtomicBool,
    stop: AtomicBool,
}

impl Flags {
    fn new() -> Self {
        Self {
            pause: AtomicBool::new(false),
            stop: AtomicBool::new(false),
        }
    }

    pub fn paused(&self) -> bool {
        self.pause.load(Ordering::Relaxed)
    }

    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    pub fn pause(&self) {
        self.pause.store(true, Ordering::Relaxed);
    }

    pub fn resume(&self) {
        self.pause.store(false, Ordering::Relaxed);
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Speed over a window rather than per chunk: a chunk is whatever the network
/// delivered, so dividing it by its own arrival time produces a figure that jumps
/// between a crawl and a gigabit.
struct Meter {
    at: Instant,
    bytes: u64,
    speed: u64,
}

impl Meter {
    fn new() -> Self {
        Self {
            at: Instant::now(),
            bytes: 0,
            speed: 0,
        }
    }

    fn sample(&mut self, received: u64) {
        let elapsed = self.at.elapsed().as_secs_f64();
        if elapsed < 0.4 {
            return;
        }
        let instant = received.saturating_sub(self.bytes) as f64 / elapsed;
        self.speed = if self.speed == 0 {
            instant as u64
        } else {
            (self.speed as f64 * 0.6 + instant * 0.4) as u64
        };
        self.at = Instant::now();
        self.bytes = received;
    }
}

struct Job {
    view: JobView,
    flags: std::sync::Arc<Flags>,
    meter: Meter,
}

pub struct Jobs(Mutex<Vec<Job>>);

impl Jobs {
    pub fn new() -> Self {
        Self(Mutex::new(Vec::new()))
    }

    pub fn list(&self) -> Vec<JobView> {
        match self.0.lock() {
            Ok(jobs) => jobs.iter().map(|job| job.view.clone()).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Adds a job, or answers with the id of the one already in the list.
    ///
    /// A second press on the same row must not start a second transfer: the two
    /// would write to the same `.part` file and neither would finish.
    pub fn add(
        &self,
        kind: Kind,
        repo: &str,
        path: &str,
        name: &str,
        destination: &str,
    ) -> (String, std::sync::Arc<Flags>) {
        let id = format!("{}/{}/{}", kind.folder(), repo, path);
        let flags = std::sync::Arc::new(Flags::new());
        let mut jobs = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        if let Some(existing) = jobs.iter().find(|job| job.view.id == id) {
            if existing.view.state == JobState::Running || existing.view.state == JobState::Paused {
                return (id, std::sync::Arc::clone(&existing.flags));
            }
        }
        // A finished or failed job of the same file is replaced rather than kept
        // beside the new one: two rows with one id is a list that cannot answer
        // "is this downloading".
        jobs.retain(|job| job.view.id != id);
        jobs.insert(
            0,
            Job {
                view: JobView {
                    id: id.clone(),
                    kind,
                    repo: repo.to_string(),
                    path: path.to_string(),
                    name: name.to_string(),
                    destination: destination.to_string(),
                    state: JobState::Running,
                    received: 0,
                    total: None,
                    speed: 0,
                    error: None,
                },
                flags: std::sync::Arc::clone(&flags),
                meter: Meter::new(),
            },
        );
        (id, flags)
    }

    pub fn flags(&self, id: &str) -> Option<std::sync::Arc<Flags>> {
        let jobs = self.0.lock().ok()?;
        jobs.iter()
            .find(|job| job.view.id == id)
            .map(|job| std::sync::Arc::clone(&job.flags))
    }

    /// Puts a paused job back to work and answers with what to restart.
    ///
    /// The flags are reused rather than replaced: a task that was still reading
    /// when the pause was pressed must see the same flag clear.
    pub fn resume(&self, id: &str) -> Option<JobView> {
        let mut jobs = self.0.lock().ok()?;
        let job = jobs.iter_mut().find(|job| job.view.id == id)?;
        if job.view.state != JobState::Paused {
            return None;
        }
        job.flags.resume();
        job.view.state = JobState::Running;
        job.view.error = None;
        job.meter = Meter::new();
        Some(job.view.clone())
    }

    pub fn progress(&self, id: &str, received: u64, total: Option<u64>) {
        let Ok(mut jobs) = self.0.lock() else { return };
        let Some(job) = jobs.iter_mut().find(|job| job.view.id == id) else {
            return;
        };
        job.view.received = received;
        job.view.total = total;
        job.meter.sample(received);
        job.view.speed = job.meter.speed;
    }

    pub fn state(&self, id: &str, state: JobState, error: Option<String>) {
        let Ok(mut jobs) = self.0.lock() else { return };
        let Some(job) = jobs.iter_mut().find(|job| job.view.id == id) else {
            return;
        };
        job.view.state = state;
        job.view.error = error;
        if state != JobState::Done {
            job.view.speed = 0;
        }
    }

    /// Marks a job for pausing. The stream loop notices between chunks.
    pub fn pause(&self, id: &str) -> bool {
        let Ok(jobs) = self.0.lock() else {
            return false;
        };
        let Some(job) = jobs.iter().find(|job| job.view.id == id) else {
            return false;
        };
        if job.view.state != JobState::Running {
            return false;
        }
        job.flags.pause();
        true
    }

    /// Marks a job for stopping, which also discards the partial file.
    pub fn stop(&self, id: &str) -> bool {
        let Ok(jobs) = self.0.lock() else {
            return false;
        };
        let Some(job) = jobs.iter().find(|job| job.view.id == id) else {
            return false;
        };
        if job.view.state != JobState::Running && job.view.state != JobState::Paused {
            return false;
        }
        job.flags.stop();
        true
    }
}
