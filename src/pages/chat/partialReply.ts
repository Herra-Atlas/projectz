/**
 * What a reply that ended early leaves in the transcript.
 *
 * Extracted because three paths end a run without the backend's `completed` event
 * -- the user pressing stop, a stop raised elsewhere reaching the listener, and
 * the page unmounting while a reply is still going -- and all three have to write
 * the same thing. It is the same reason [`closeOpenSteps`](./closeOpenSteps.ts)
 * exists: one rule, applied by every path, so a fourth path cannot get it wrong by
 * omission.
 *
 * [`closeOpenSteps`]: ./closeOpenSteps.ts
 */

import type { ChatMessage } from "../../features/chat/types";
import type { BackgroundRun } from "./types";

/**
 * Whether a run has anything worth writing down.
 *
 * Text **or** steps, and the "or" is the whole point. This was once "did it stream
 * any text", which quietly threw away every run stopped during the work: the
 * answer is the last thing a reply produces, so a stop while the agent was reading
 * files had no text yet -- the partially-filled steps were dropped and the live
 * panel was cleared, and the user watched the work they had just cancelled
 * disappear from the screen.
 *
 * A run that called a tool did something. That is the record, and it is kept.
 */
export function hasPartialReply(run: BackgroundRun): boolean {
  return run.text.length > 0 || run.activity.length > 0;
}

/**
 * The assistant message a partial run leaves behind.
 *
 * Content may be empty, which is normal here: a run stopped mid-tool has no prose,
 * and an empty string is a truthful answer for a reply that never got to write
 * one. The steps are where the substance is.
 *
 * No `metrics`, so the panel falls back to summing the steps for its "Worked for"
 * figure rather than reporting a whole-reply duration that never happened. The
 * step sum is the honest figure for a run that was cut off: each step's own
 * measured seconds, and nothing invented for the time nobody measured.
 */
export function partialReplyMessage(run: BackgroundRun): ChatMessage {
  return {
    role: "assistant",
    content: run.text,
    reasoning: run.reasoning || undefined,
    ...(run.activity.length > 0 ? { activity: run.activity } : {}),
    modelId: run.modelLabel,
    providerId: run.providerLabel,
  };
}
