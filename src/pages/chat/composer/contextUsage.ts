/**
 * How full the context window is, and whether the figure is measured.
 *
 * Extracted from `ChatPage` because it is arithmetic rather than state: the page
 * has the pieces and this decides what they add up to. Kept in a file of its own
 * so it can be reasoned about (and changed) without reading the whole chat shell.
 *
 * # What "tokens in the request" means
 *
 * The number the user cares about is *how big the next request will be*, not how
 * much has been billed and not what the last reply happened to be. Three sources,
 * in order of trust:
 *
 * 1. **The provider's own count for the round in flight.** A tool round reports the
 *    real prompt size, and this is the only measurement of context that exists
 *    while a reply is running. It is the base whenever it is there -- see
 *    `livePromptTokens`, which the backend now emits per round.
 * 2. **The last stored reply's `prompt_tokens`.** Exact for what it measured, and
 *    a round or two out of date by the time it is read.
 * 3. **Characters over four.** A guess, and labelled as one.
 *
 * On top of whichever base applies, everything appended since it was measured is
 * estimated from its length: the turns added after that reply, the tool calls and
 * results recorded in the steps, and the text streaming now. Nothing here invents
 * a number it cannot support -- if there is no base, the whole figure is marked an
 * estimate and the ring says so.
 */

import type { ActivityStep, ChatMessage } from "../../../features/chat/types";

/** Characters per token, for every figure this file estimates. */
const CHARS_PER_TOKEN = 4;

export type ContextUsage = {
  tokens: number;
  /** True when there was no measured base at all and the whole figure is a guess. */
  estimated: boolean;
};

export type ContextUsageInputs = {
  /** The stored transcript, oldest first. */
  messages: ChatMessage[];
  /**
   * The prompt size the provider reported for the round in flight.
   *
   * `null` when no round of the current run has reported one yet, which is the
   * case for a provider that sends no usage and for the moment before the first
   * round completes.
   */
  livePromptTokens: number | null;
  /** The steps of the run in flight, whose calls and results are part of the next request. */
  activity: ActivityStep[];
  /** The answer text streaming now. */
  streamText: string;
  /** The thinking streaming now, which is also in the request. */
  streamReasoning: string;
};

/** The last measured prompt size in the transcript, and where it stopped counting. */
function lastStoredReport(messages: ChatMessage[]): { tokens: number; after: number } {
  // Walked backwards rather than with `findLastIndex`, which needs a newer
  // language target than this app compiles to.
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const tokens = messages[index].metrics?.prompt_tokens;
    if (tokens !== undefined) return { tokens, after: index + 1 };
  }
  return { tokens: 0, after: 0 };
}

/** The characters one stored message contributes to a request. */
function messageCharacters(message: ChatMessage): number {
  // An old web-search `tool` message carries its results outside `content`.
  const tool = message.tool;
  const results = tool
    ? tool.results.reduce((total, result) => total + result.title.length + result.snippet.length, 0)
    : 0;
  return message.content.length + results;
}

/** The characters the steps of a run contribute: what was asked for, and what came back. */
function activityCharacters(activity: ActivityStep[]): number {
  let total = 0;
  for (const step of activity) {
    // A tool contributes what it was asked for and what came back; a thought and a
    // note are text. Both are in the request either way -- the steps are the only
    // record of a run that has not been stored yet.
    if (step.kind === "tool") total += step.detail.length + (step.output?.length ?? 0);
    else total += step.text.length;
  }
  return total;
}

/** Rounds a character count to tokens. */
function tokensFor(characters: number): number {
  return Math.ceil(characters / CHARS_PER_TOKEN);
}

export function contextUsage({
  messages,
  livePromptTokens,
  activity,
  streamText,
  streamReasoning,
}: ContextUsageInputs): ContextUsage {
  const streaming = tokensFor(streamText.length + streamReasoning.length);

  // The freshest measurement wins, and a live one is fresher than anything stored:
  // it counts the request the provider is working on right now.
  if (livePromptTokens !== null) {
    return { tokens: livePromptTokens + streaming, estimated: false };
  }

  const report = lastStoredReport(messages);
  if (report.tokens > 0) {
    // Measured base, estimated addition: the turns added since that reply, and the
    // text streaming now. The steps of this run are deliberately not added here --
    // they have not been sent yet, and the next round's report will count them.
    const added = messages
      .slice(report.after)
      .reduce((total, message) => total + messageCharacters(message), 0);
    return { tokens: report.tokens + tokensFor(added) + streaming, estimated: false };
  }

  // Nothing has ever reported usage, so the whole figure is an inference from
  // length -- and it has to count everything, because nothing has been counted for
  // it. Labelled as an estimate wherever it is drawn.
  const everything =
    messages.reduce((total, message) => total + messageCharacters(message), 0) +
    activityCharacters(activity);
  return { tokens: tokensFor(everything) + streaming, estimated: true };
}
