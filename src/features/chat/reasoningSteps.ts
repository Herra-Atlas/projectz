/**
 * The two folds that turn reasoning events into thought rows.
 *
 * Both listeners that build an activity list -- the chat page's own and the
 * sub-agent panel's -- receive the same `reasoning_delta` / `reasoning_segment`
 * pair, so the rule that turns them into rows lives here rather than being
 * written twice and drifting apart.
 */

import type { ActivityStep, ThoughtStep } from "./types";

/**
 * Appends a streamed stretch of thinking to the open thought, or opens one.
 *
 * A delta always continues the row above it when that row is still thinking,
 * and starts a new row otherwise: a thought that began before a tool call stays
 * its own step instead of merging with the thinking that came after it.
 */
export function appendReasoning(steps: ActivityStep[], text: string, now = Date.now()): void {
  const last = steps[steps.length - 1];
  if (last?.kind === "thought" && last.running) {
    last.text += text;
  } else {
    steps.push({ kind: "thought", text, running: true, startedAt: now });
  }
}

/**
 * Settles a thought the backend has closed.
 *
 * The text arrives whole, so it replaces whatever streamed in its place.
 *
 * **The row it replaces may already be closed.** A thought stops when the
 * answer begins (`reasoning_phase`), which arrives *before* the segment that
 * reports it closed -- so by the time this lands, the row above is a finished
 * thought with the same text rather than a running one. Matching only on
 * `running` pushed a second copy of the same thought beside the first, with its
 * duration lost because the first had already been timed. Text is the identity
 * here: the segment is built from exactly the deltas that produced that row, so
 * an equal string is the same thought, and a different one is a new row.
 */
export function applyReasoningSegment(steps: ActivityStep[], text: string, now = Date.now()): void {
  const last = steps[steps.length - 1];
  if (last?.kind === "thought" && (last.running || last.text === text)) {
    steps[steps.length - 1] = settledThought(last, text, now);
    return;
  }
  steps.push({ kind: "thought", text, seconds: 0, running: false });
}

/**
 * The closed row: the backend's text, and the duration measured when it ended.
 *
 * A thought the phase change already closed keeps its figure -- that is when the
 * thinking actually stopped, and its start timestamp was dropped at that moment.
 */
function settledThought(step: ThoughtStep, text: string, now: number): ThoughtStep {
  const seconds =
    step.startedAt !== undefined ? Math.max(0, (now - step.startedAt) / 1000) : (step.seconds ?? 0);
  return { kind: "thought", text, seconds, running: false };
}
