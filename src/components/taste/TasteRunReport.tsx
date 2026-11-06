import { AlertCircle } from "lucide-react";
import type { TasteRun } from "../../features/taste/types";
import { dateTime } from "../../features/taste/tasteFormat";

/** The status tones, so a completed and a failed run do not look the same. */
const STATUS_TONE: Record<string, string> = {
  completed: "text-[var(--accent)]",
  failed: "text-[var(--danger)]",
  skipped: "text-[var(--quiet)]",
};

function changeSummary(run: TasteRun): string {
  const parts: string[] = [];
  if (run.added) parts.push(`${run.added} added`);
  if (run.updated) parts.push(`${run.updated} updated`);
  if (run.removed) parts.push(`${run.removed} removed`);
  return parts.length ? parts.join(" · ") : "No change";
}

/**
 * One run: what it cost, what it changed, and what the model said about it.
 *
 * Deliberately not a second profile view. The profile is learnt state; this is the
 * record of one attempt, which is what someone reads when a fact looks wrong or a
 * run failed without explaining itself.
 */
export default function TasteRunReport({ run }: { run: TasteRun }) {
  const tone = STATUS_TONE[run.status] ?? "text-[var(--muted)]";
  return (
    <div className="px-4 py-4">
      <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
        {/* The reason is a hover fact, not a paragraph: a failed run row is a
            summary like every other row, and a full error sentence under it made
            the latest run taller than the list it belongs to. The dotted
            underline is the affordance that says there is more on hover. */}
        <span
          className={`text-[12px] font-semibold capitalize ${tone} ${
            run.reason ? "cursor-help underline decoration-dotted underline-offset-2" : ""
          }`}
          title={run.reason ?? undefined}
        >
          {run.status}
        </span>
        {run.reason && run.status === "failed" && (
          <AlertCircle size={12} className="shrink-0 self-center text-[var(--danger)]" aria-hidden />
        )}
        <span className="text-[11px] text-[var(--quiet)]">{dateTime(run.startedAt)}</span>
        <span className="text-[11px] text-[var(--quiet)]">
          {run.sessionsRead} {run.sessionsRead === 1 ? "conversation" : "conversations"} ·{" "}
          {run.messagesRead} {run.messagesRead === 1 ? "turn" : "turns"}
        </span>
        {(run.promptTokens != null || run.completionTokens != null) && (
          <span className="text-[11px] text-[var(--quiet)]">
            {run.promptTokens ?? "?"} in · {run.completionTokens ?? "?"} out
          </span>
        )}
        {/* The same right-aligned change count the history rows use. It belongs on
            the summary line, not a line of its own. */}
        <span className="ml-auto shrink-0 text-[12px] text-[var(--muted)]">{changeSummary(run)}</span>
      </div>

      {run.report && (
        <p className="mt-2 whitespace-pre-wrap text-[12px] leading-5 text-[var(--text)]">{run.report}</p>
      )}

      {run.document && (
        <details className="mt-3 border-t border-[var(--line)] pt-2">
          <summary className="cursor-pointer text-[11px] text-[var(--quiet)] hover:text-[var(--muted)]">
            Profile at the end of this run
          </summary>
          <pre className="mt-2 max-h-56 overflow-auto whitespace-pre-wrap rounded-md bg-[var(--rail)] p-3 text-[11px] leading-5 text-[var(--muted)]">
            {run.document}
          </pre>
        </details>
      )}
    </div>
  );
}
