pub mod local;
pub mod prompts;
pub mod remote;
pub mod runtime;
pub mod tools;
pub mod types;

/// What a stopped run reports, so a stop is not mistaken for a failure.
///
/// The frontend shows a `Request failed: ...` bubble for any error, so a stop
/// reported as an error would tell the user something broke when they did the
/// breaking on purpose. Compared by equality in the runtime, which is why it is a
/// fixed string rather than an error type.
pub const STOPPED: &str = "Stopped";
