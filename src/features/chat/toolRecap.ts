import type { ActivityStep, ChatMessage } from "./types";

/**
 * What a reply's tools actually returned, carried on that reply's own text.
 *
 * # Why this exists
 *
 * The provider is only ever sent `{role, content}` -- `prepare_messages` in the
 * backend drops every other field -- so the steps behind a reply never leave this
 * app. That is right for the panel, which draws them, and wrong for the model: on
 * the next turn it remembers what it *said* and has no record of what it *saw*.
 * A conversation that used the browser could not explain work it had done one turn
 * earlier, which is exactly what happened.
 *
 * # Why it is derived rather than stored
 *
 * The same stored activity always produces the same text, so the bytes of the
 * request prefix do not change between turns and a provider's prompt cache still
 * matches. A separately stored summary would drift from the steps it summarises,
 * and one that changes would invalidate every token after it.
 *
 * # Why it is budgeted
 *
 * A page tree is large, and an old one is worth a glance rather than a re-read.
 * Each result keeps its opening -- where a tool's own summary lives -- and a reply
 * stops adding results once its budget is spent.
 */

/** Characters kept from one tool result. */
const RESULT_BUDGET = 1200;

/** Characters kept from one reply's whole set of results. */
const REPLY_BUDGET = 4_000;

/** The marker that separates a reply's text from the record of its work. */
const MARKER = "[What I did in that reply]";

export function withToolRecap(message: ChatMessage): ChatMessage {
  if (message.role !== "assistant" || !message.activity?.length) return message;
  // Already marked, which can only happen if this ran twice; adding a second
  // record would grow the message on every turn and destroy the cache.
  if (message.content.includes(MARKER)) return message;

  const steps = message.activity.filter(isTool);
  if (steps.length === 0) return message;

  const lines = ["", "", MARKER];
  let spent = 0;
  for (const step of steps) {
    lines.push(`- ${step.label}${step.detail ? `: ${step.detail}` : ""}${step.failed ? " (failed)" : ""}`);
    const body = (step.output ?? "").trim() || (step.outcome ?? "").trim();
    if (!body) continue;
    if (spent >= REPLY_BUDGET) {
      lines.push("  … the rest of this reply's output is omitted.");
      break;
    }
    const room = Math.min(RESULT_BUDGET, REPLY_BUDGET - spent);
    lines.push(indent(clip(body, room)));
    spent += Math.min(body.length, room);
  }
  return { ...message, content: `${message.content}${lines.join("\n")}` };
}

type ToolStep = Extract<ActivityStep, { kind: "tool" }>;

function isTool(step: ActivityStep): step is ToolStep {
  return step.kind === "tool";
}

function indent(text: string): string {
  return text
    .split("\n")
    .map((line) => `  ${line}`)
    .join("\n");
}

function clip(text: string, limit: number): string {
  return text.length <= limit ? text : `${text.slice(0, limit)}…`;
}
