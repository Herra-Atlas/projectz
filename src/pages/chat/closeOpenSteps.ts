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