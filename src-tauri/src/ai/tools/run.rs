//! What a tool is told about the run it belongs to.
//!
//! A tool's executor is a bare `fn(Value, ToolContext) -> Future`, so it cannot
//! capture anything from the loop that called it. Everything a tool needs about
//! *the run* -- which model and endpoint it is working against, how much it may
//! do without asking, whether the model is local, and how to reach the user's
//! screen -- therefore has to travel in [`ToolContext`] rather than be captured.
//!
//! This is deliberately the same reasoning as [`super::workspace`]: threading a
//! run-scoped value through the context is the honest shape, and the context is
//! the one place a bare function pointer can read it from.
//!
//! [`ToolContext`]: super::ToolContext

use std::sync::Arc;

use crate::ai::remote::types::Endpoint;
use crate::ai::types::{ChatEvent, ReasoningEffort};

use super::{ApprovalGate, ToolMode};

/// The run a tool call belongs to.
///
/// Held as one `Option<Arc<..>>` on the context rather than as a handful of
/// separate fields, for two reasons. A tool that does not need the run still
/// reads a context that says nothing about a model, and a tool that *does* --
/// today only `spawn_agent` -- takes one clone and has everything. `None` means
/// the call has no run behind it, which is the case in a tool's own unit tests
/// and is reported as a clean tool error rather than a panic.
pub struct RunHandle {
    /// The endpoint the parent run is talking to. A sub-agent defaults to this
    /// one, so a delegated task stays on the provider the user chose.
    pub endpoint: Endpoint,
    /// The model the parent run is using. The fallback when neither the call nor
    /// the stored preference names a sub-agent model.
    pub model: String,
    /// The reasoning level to hand a sub-agent. Carried rather than defaulted so
    /// a sub-agent reasons the way its parent does.
    pub reasoning: ReasoningEffort,
    /// Whether web search is on for this conversation, so a sub-agent can be
    /// given the same tools its parent has.
    pub web_search_enabled: bool,
    /// The tool set the parent is running with. A sub-agent runs in Agent mode
    /// regardless, because it exists to *do* things.
    pub mode: ToolMode,
    /// Passed straight through to the sub-agent's requests, matching whatever
    /// the parent sent (true only for a local model).
    pub enable_reasoning_control: bool,
    /// Whether the parent is a local model.
    ///
    /// Load-bearing for concurrency: the bundled `llama-server` defaults to a
    /// single slot, so two sub-agents firing requests at it at once would queue
    /// or fail. A local run therefore fans out sequentially even when the round
    /// asked for several agents.
    pub local: bool,
    /// The parent's run id. An approval raised inside a sub-agent is forwarded to
    /// the frontend under *this* id, because it is the only run the screen knows.
    pub run_id: String,
    /// The parent's conversation, used to scope a sub-agent's tool cache so a
    /// write it makes invalidates its parent's earlier cached reads.
    pub session_id: Option<String>,
    /// The permission gate, shared by clone so a sub-agent's calls are checked
    /// against exactly the same mode and pending-prompt map as its parent's.
    pub approval: ApprovalGate,
    /// Emits an event to the UI. Shared rather than borrowed so several
    /// sub-agents running at once can each raise an approval prompt.
    pub emit: Arc<dyn Fn(ChatEvent) + Send + Sync>,
    /// The model a sub-agent should use when the call does not name one.
    ///
    /// Resolved once by the runtime from the `app.preferences` sub-agent model,
    /// so a local selection has already been turned into a live endpoint here
    /// and the tool never has to reach for the model registry itself.
    pub subagent_default: Option<(Endpoint, String)>,
}

/// One sub-agent reporting that it has finished, so the panel can refresh.
///
/// A notice rather than the transcript: the run is already written to the
/// database by the time this fires, and the panel reads it from there. Sending
/// the whole transcript through the event would put a second copy of it on the
/// wire for a tab that may not even be open.
#[derive(Clone, Debug)]
pub struct SubAgentNotice {
    pub id: String,
    pub label: String,
    pub session_id: Option<String>,
}
