pub mod compact;
pub mod local;
pub mod prompts;
pub mod recap;
pub mod remote;
pub mod run;
pub mod runtime;
pub mod subagent;
pub mod tools;
pub mod types;

/// What a stopped run reports, so a stop is not mistaken for a failure.
///
/// The frontend shows a `Request failed: ...` bubble for any error, so a stop
/// reported as an error would tell the user something broke when they did the
/// breaking on purpose. Compared by equality in the runtime, which is why it is a
/// fixed string rather than an error type.
pub const STOPPED: &str = "Stopped";

/// How long to wait before each retry of a failed request, in seconds.
///
/// One ladder for every kind of request: a reply, a job, a message from a phone and
/// a taste batch. A connection error is usually gone in a second, and a provider
/// having a bad minute is worth about fifteen seconds of patience. Four waits, so
/// five attempts at most.
///
/// **A retry is only offered to work that changed nothing.** For a chat run that
/// means no token and no tool; for a short internal request it means no usable
/// answer. Neither can be duplicated by re-sending, which is the whole reason a
/// retry is safe here at all.
pub const RETRY_DELAYS_SECONDS: [u64; 4] = [1, 3, 5, 7];
