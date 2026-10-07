import type { ActivityStep } from "../chat/types";
import type { AiEvent, WebSearchOutput } from "../../pages/chat/types";

/**
 * A sub-agent run while it is still happening.
 *
 * A live run folds into the same shape the stored transcript does -- see
 * `src-tauri/src/ai/subagent/transcript.rs` -- so the panel can draw a run in
 * progress with the same components it draws a finished one with: a task, the
 * activity steps behind the answer, and the answer itself. The difference is
 * where the facts come from. A finished run is read back from a saved row; a
 * live one is built here from the events the backend forwards as the run
 * streams, because there is no row yet.
 */
export type LiveRun = {
  id: string;
  title: string;
  /** The conversation that spawned this run, so the panel can scope the list. */
  parentSessionId: string | null;
  /** The brief the agent was given, shown above its answer. */
  prompt: string;
  modelId: string;
  providerId: string;
  startedAt: string;
  activity: ActivityStep[];
  text: string;
  reasoning: string;
  searching: boolean;
};

/**
 * A running sub-agent as the backend reports it.
 *
 * The same shape the `ai_running_subagents` command returns and the start event
 * carries, so a run discovered either way becomes the same `LiveRun` through
 * [`createLiveRun`].
 */
export type RunningSubAgent = {
  id: string;
  label: string;
  sessionId: string | null;
  prompt: string;
  model: string;
  provider: string;
  startedAt: string;
};

/**
 * A short title for a run, matching `title_from` in the transcript builder: the
 * label when the model gave one, otherwise the first line of the task. Kept in
 * step with the Rust so a run's name does not change when it moves from memory
 * to storage.
 */
function titleFor(label: string, prompt: string): string {
  if (label.trim()) return label;
  const first = (prompt.split("\n")[0] ?? "").trim().slice(0, 48);
  return first || "Sub-agent";
}

/** A fresh live run from the seed the backend reports at start. */
export function createLiveRun(seed: RunningSubAgent): LiveRun {
  return {
    id: seed.id,
    title: titleFor(seed.label, seed.prompt),
    parentSessionId: seed.sessionId,
    prompt: seed.prompt,
    modelId: seed.model,
    providerId: seed.provider,
    startedAt: seed.startedAt,
    activity: [],
    text: "",
    reasoning: "",
    searching: false,
  };
}

/**
 * The seed carried by a `subagent` start event.
 *
 * `null` when the event names no agent, which a hand-built payload could do; the
 * caller skips it rather than adding a run it cannot identify.
 */
export function runningSubAgentFromEvent(metrics: AiEvent["metrics"]): RunningSubAgent | null {
  const id = metrics?.agent_id;
  if (!id) return null;
  return {
    id,
    label: metrics?.label ?? "",
    sessionId: metrics?.session_id ?? null,
    prompt: metrics?.prompt ?? "",
    model: metrics?.model ?? "",
    provider: metrics?.provider ?? "",
    startedAt: metrics?.started_at ?? new Date().toISOString(),
  };
}

/**
 * Folds one streamed event into a live run, in place.
 *
 * A mirror of the chat page's own listener, for the same reason
 * `transcript.rs` mirrors it: the shapes are shared (`ActivityStep`, the step
 * kinds) and the panel draws them the same way, so the folding has to produce
 * the same thing. Deliberately not shared with the chat page -- that listener is
 * bound to React state, the background-run map and the notification stack, none
 * of which a sub-agent's live view has or wants.
 */
export function foldEvent(run: LiveRun, event: AiEvent): void {
  const metrics = event.metrics ?? {};
  switch (event.kind) {
    case "status":
      run.searching = event.text === "searching";
      break;
    case "reasoning_delta":
      if (event.text) {
        const last = run.activity[run.activity.length - 1];
        if (last?.kind === "thought" && last.running) {
          last.text += event.text;
        } else {
          run.activity.push({ kind: "thought", text: event.text, running: true, startedAt: Date.now() });
        }
        run.reasoning += event.text;
      }
      break;
    case "reasoning_segment":
      if (event.text) {
        const last = run.activity[run.activity.length - 1];
        const seconds = last?.kind === "thought" && last.startedAt !== undefined
          ? (Date.now() - last.startedAt) / 1000
          : 0;
        const segment: ActivityStep = { kind: "thought", text: event.text, seconds, running: false };
        if (last?.kind === "thought" && last.running) run.activity[run.activity.length - 1] = segment;
        else run.activity.push(segment);
      }
      break;
    case "tool_call":
      if (event.metrics) {
        run.activity.push({
          kind: "tool",
          tool: event.text ?? "tool",
          label: metrics.label ?? "Tool call",
          detail: metrics.detail ?? "",
          outcome: "",
          failed: false,
          running: true,
          startedAt: Date.now(),
        });
      }
      break;
    case "tool_result": {
      const open = [...run.activity]
        .reverse()
        .find((step) => step.kind === "tool" && step.running && step.tool === (event.text ?? "tool"));
      if (open && open.kind === "tool") {
        open.running = false;
        open.seconds = metrics.seconds;
        open.outcome = metrics.outcome ?? "";
        open.failed = metrics.failed === true;
        if (metrics.cached) open.cached = true;
        if (metrics.output) open.output = metrics.output;
        if (metrics.diff) open.diff = metrics.diff;
        if (metrics.created) open.created = true;
        delete open.startedAt;
      } else {
        // A result with no open row is still shown, for the same reason the chat
        // panel shows it: a call the user cannot see is one they cannot account for.
        run.activity.push({
          kind: "tool",
          tool: event.text ?? "tool",
          label: metrics.label ?? "Tool call",
          detail: metrics.detail ?? "",
          outcome: metrics.outcome ?? "",
          failed: metrics.failed === true,
          seconds: metrics.seconds,
          running: false,
          ...(metrics.cached ? { cached: true } : {}),
          ...(metrics.output ? { output: metrics.output } : {}),
          ...(metrics.diff ? { diff: metrics.diff } : {}),
          ...(metrics.created ? { created: true } : {}),
        });
      }
      break;
    }
    case "web_search":
      if (event.search) attachSearch(run.activity, event.search);
      break;
    case "web_search_failed":
      attachSearch(run.activity, { query: event.text ?? "", results: [] });
      break;
    case "delta":
      if (event.text) run.text += event.text;
      break;
    default:
      break;
  }
}

/**
 * Records a web search's results on the `search_web` step that asked for them.
 *
 * The step is found rather than created because the call's own row already
 * exists; results arriving with no open search are appended as their own step,
 * because a search that ran and cannot be seen is worse than an extra row. The
 * same rule the chat page uses, so the two panels show the same thing.
 */
function attachSearch(steps: ActivityStep[], search: WebSearchOutput): void {
  const open = [...steps].reverse().find((step) => step.kind === "tool" && step.tool === "search_web");
  if (open && open.kind === "tool") {
    open.search = search.results.length > 0 ? search : { query: search.query, results: [] };
  } else {
    steps.push({
      kind: "tool",
      tool: "search_web",
      label: "Search the web",
      detail: search.query,
      outcome: search.results.length > 0 ? `${search.results.length} results` : "No results found",
      failed: false,
      running: false,
      search,
    });
  }
}
