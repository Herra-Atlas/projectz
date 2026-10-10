/**
 * Wire shapes the chat page speaks in.
 *
 * These were declared at the top of `ChatPage.tsx` and are here so the run
 * listener, the composer and the page shell can all name one thing. They are the
 * transport contracts: what the backend emits over `ai-event`, and what a
 * background run holds while it streams. Nothing here owns state.
 */

import type { ActivityStep, ChatMessage, ToolDiff } from "../../features/chat/types";
import type { ModelSelection } from "../../features/models/types";

/**
 * One payload of the `ai-event` stream.
 *
 * Deliberately open — `kind` is a plain string and `metrics` is a bag of
 * optionals, because the backend emits a different subset for each kind. A
 * closed union here would need a variant per event and would reject a field the
 * next backend change adds, which is exactly the kind of failure this page
 * cannot afford mid-reply.
 */
export type AiEvent = {
  run_id: string;
  session_id?: string | null;
  sequence?: number;
  kind: string;
  text?: string;
  error?: string;
  search?: WebSearchOutput;
  query?: string;
  metrics?: {
    approval_id?: string;
    tool?: string;
    command?: string;
    arguments?: Record<string, unknown>;
    prompt_tokens?: number;
    completion_tokens?: number;
    cached_tokens?: number;
    prompt_eval_tokens_per_second?: number;
    generation_tokens_per_second?: number;
    index?: number;
    running?: boolean;
    label?: string;
    detail?: string;
    outcome?: string;
    failed?: boolean;
    seconds?: number;
    cached?: boolean;
    output?: string;
    diff?: ToolDiff;
    created?: boolean;
    /**
     * The `ask_user` prompt: the id the answer is keyed on, the question, and
     * any suggested answers offered as buttons.
     */
    question_id?: string;
    question?: string;
    options?: string[];
    /** The agent's checklist, carried on a `todo` event. */
    items?: { text: string; status: string }[];
    /**
     * The sub-agent lifecycle fields, carried on a `subagent` event.
     *
     * `agent_id` names the run, `state` is `running` or `done`, and the rest are
     * the facts a panel needs to show a run that has no stored row yet -- the task
     * it was given, and the model it runs on.
     */
    agent_id?: string;
    state?: string;
    session_id?: string | null;
    prompt?: string;
    model?: string;
    provider?: string;
    started_at?: string;
  };
};

/** A finished `search_web`, attached to its step rather than sent as a message. */
export type WebSearchOutput = {
  query: string;
  results: { title: string; url: string; snippet: string }[];
};

/** One of the three payloads `local-model-event` carries. */
export type LocalModelEvent = {
  kind: "loading" | "loaded" | "failed";
  modelId?: string;
  message?: string;
};

/**
 * A send parked on the "start llama.cpp?" prompt.
 *
 * Held across the pause rather than re-derived when the engine comes up: the
 * skill was chosen for this message, and it is still that message when the
 * server finally starts. Reading a later state at resume time would drop it.
 */
export type PendingChat = {
  messages: ChatMessage[];
  sessionId: string;
  attachments: AttachedFile[];
  skillIds: string[];
};

/** A dropped or picked file, already read and base64'd by the backend. */
export type AttachedFile = {
  path: string;
  name: string;
  mime_type: string;
  data_base64: string;
};

/**
 * One in-flight reply.
 *
 * At module scope rather than inside the hook because `attachSearch` names it:
 * the helper needs the run's activity, and a type declared inside a function body
 * would put that helper out of reach.
 */
export type BackgroundRun = {
  sessionId: string;
  baseMessages: ChatMessage[];
  text: string;
  reasoning: string;
  /**
   * The steps behind this run, in order.
   *
   * On the run rather than in state because a run keeps streaming while the user
   * is reading another session, and the steps have to still be there when they
   * come back. Mirrors into the visible stream only while this session is the one
   * on screen.
   */
  activity: ActivityStep[];
  searching: boolean;
  modelLabel: string;
  providerLabel?: string;
  /**
   * When the run's *first* request went out, in `performance.now()` terms.
   *
   * Zero until the first `thinking` status arrives, rather than being set at
   * construction: the backend emits one status per tool round, so this is what
   * distinguishes the first from the rest. `elapsed_seconds` is measured against
   * it, so a value overwritten per round makes the panel report the last round
   * rather than the run.
   */
  requestStartedAt: number;
  generationStartedAt: number;
  completionId?: string;
  localModel: boolean;
  isReasoning: boolean;
};

/** Text the user highlighted inside a reply, with where to anchor the toolbar. */
export type SelectedText = {
  text: string;
  top: number;
  left: number;
};

/**
 * Stable identity so an absent prop does not retrigger the title-model effect.
 *
 * A fresh `[]` literal on every render would be a new reference each render,
 * which is a dependency change on every render, so the effect would never settle.
 * Module scope rather than `useMemo` because the point is that it never changes.
 */
export const EMPTY_TITLE_MODELS: ModelSelection[] = [];