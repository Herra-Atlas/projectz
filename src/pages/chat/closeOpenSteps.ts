/**
 * Closing the steps a run leaves open, and the one pure step transform in the
 * chat page.
 *
 * Extracted because three separate paths end a run — completion, failure and the
 * user pressing stop — and each one has to apply the same rule. Keeping the rule
 * in one function is what makes a fourth path impossible to get wrong by
 * omission: it calls the same thing.
 */

import type { ActivityStep } from "../../features/chat/types";

/**
 * The shortest thinking worth giving a row of its own.
 *
 * Mirrors `MIN_REPORTED_REASONING_CHARS` in `client.rs`. The backend uses it to
 * decide whether to report a closed segment at all, and the frontend uses it to
 * decide whether a row is worth keeping when a run ends without one -- so the
 * two agree on where the line is, and a thought just under it is dropped by the
 * same rule in both places rather than appearing in one and not the other.
 */
export const MIN_THOUGHT_CHARS = 24;

/**
 * Closes every step a run left open, in place, and returns the steps to keep.
 *
 * The backend can only report a finished thought when a tool call or the final
 * answer proves the thinking ended, and only when the segment is long enough to
 * be worth a row. So a short thought, and any run that was stopped or failed,
 * ends with a row still marked running -- rendering as a thought that never
 * finished, counting up from a timestamp nothing will ever advance.
 *
 * Every path that ends a run goes through here: completion, failure, and the
 * user pressing stop. Leaving one of them out is how a step ends up stuck open,
 * and the symptom is a panel that looks broken rather than one that reports a
 * wrong number.
 *
 * Timing is the truth for a run that has ended -- the thinking did stop, and
 * this is when. A thought too short for a row is dropped rather than kept with
 * its text cleared, matching what the backend would have reported had it been
 * asked. A tool step is never dropped: it did something, and its row is how the
 * reader accounts for it.
 *
 * Mutates in place and returns the same array, because every caller is already
 * holding the run's own `activity` reference and the panel re-renders from it.
 */
export function closeOpenSteps(activity: ActivityStep[]): ActivityStep[] {
  const closedAt = Date.now();
  return activity.filter((step) => {
    // A note carries no clock -- it is prose the run moved out of the answer, and
    // it is complete the moment it is recorded -- so there is nothing to close.
    if (step.kind === "note") return true;
    if (!step.running) return true;
    step.running = false;
    if (step.startedAt !== undefined) {
      step.seconds = Math.max(0, (closedAt - step.startedAt) / 1000);
      // Dropped once the call is done, because `seconds` is now the measured
      // figure and a local clock on the row would let a later tick add to a
      // step that finished long ago.
      delete step.startedAt;
    }
    return step.kind !== "thought" || step.text.length >= MIN_THOUGHT_CHARS;
  });
}

/**
 * Closes the trailing thought the moment something else begins, in place.
 *
 * The same rule as [`closeOpenSteps`], applied while the run is still going. The
 * backend can only report a thought as finished when a tool call or the final
 * answer proves it ended, and only when it is long enough to be worth a row -- so
 * a thought that stopped as the model moved on was left marked `running`, with the
 * panel counting seconds from a timestamp nothing would ever advance. A row that
 * says "Thought for 1m 20s" while the model has spent that time on a tool is not a
 * slow thought; it is a clock that nobody stopped.
 *
 * Called from the two places that prove a thought is over -- a tool call starting,
 * and the answer beginning to stream -- rather than from a timer, because those
 * are the moments the fact changes.
 *
 * Only the *last* step is considered: a thought that is still running is by
 * definition the most recent thing that happened, since `reasoning_delta` appends
 * to it and anything else would have pushed a step after it.
 *
 * A thought too short for a row is dropped, exactly as it would be at the end of
 * a run, so a stray fragment never becomes a row that says nothing.
 */
export function closeRunningThought(activity: ActivityStep[]): ActivityStep[] {
  const last = activity[activity.length - 1];
  if (last?.kind !== "thought" || !last.running) return activity;
  last.running = false;
  if (last.startedAt !== undefined) {
    last.seconds = Math.max(0, (Date.now() - last.startedAt) / 1000);
    delete last.startedAt;
  }
  if (last.text.length < MIN_THOUGHT_CHARS) activity.pop();
  return activity;
}