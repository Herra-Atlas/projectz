import { LoaderCircle, Sparkles } from "lucide-react";
import { useTaste } from "../../../features/taste/useTaste";
import TasteRunReport from "../../taste/TasteRunReport";

/**
 * The right panel's view of a Taste run.
 *
 * **A run is a short, private transcript**, not a conversation: what it read,
 * which batch it asked, what changed. `progress.lines` is exactly that log, emitted
 * by the backend as each step finishes, so this view is live while the model works
 * and does not poll.
 *
 * **The stored run is drawn under the log.** While it works the lines are the
 * story; once it finishes the report adds what the log does not carry -- the
 * totals, the token cost, the profile it produced. The log is kept, so opening
 * the view after a run still shows the steps that produced it.
 *
 * The hook is the settings page's own, reused rather than re-implemented: the two
 * places must agree about whether a run is happening, and two readers of the same
 * event are cheaper than two sources of truth.
 */
export default function TasteTab() {
  const { status, error } = useTaste(true);
  const running = status?.running ?? false;
  const lines = status?.progress?.lines ?? [];
  const lastRun = status?.lastRun ?? null;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex min-h-9 shrink-0 items-center gap-2 border-b border-[var(--line)] px-3 text-[12px] text-[var(--muted)]">
        <Sparkles size={13} className={`shrink-0 ${running ? "text-[var(--accent)]" : "text-[var(--quiet)]"}`} />
        <span className="min-w-0 flex-1 truncate">Taste</span>
        <span className={`shrink-0 capitalize ${running ? "text-[var(--accent)]" : lastRun?.status === "failed" ? "text-[var(--danger)]" : "text-[var(--quiet)]"}`}>
          {running ? "Learning" : (lastRun?.status ?? "Idle")}
        </span>
        {running && <LoaderCircle size={13} className="shrink-0 animate-spin text-[var(--accent)]" />}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto" data-find-scope="" data-find-label="taste">
        {lines.length > 0 ? (
          <ol className="flex flex-col gap-2 px-4 py-4 text-[12px] leading-5">
            {lines.map((line, index) => (
              // The index is part of the key because a line can repeat: two
              // batches can both report "0 added, 0 reinforced".
              <li key={`${index}-${line}`} className="flex gap-2">
                <span aria-hidden="true" className="mt-[7px] size-1 shrink-0 rounded-full bg-[var(--quiet)]" />
                <span className="min-w-0 break-words text-[var(--muted)]">{line}</span>
              </li>
            ))}
            {running && (
              <li className="flex gap-2 text-[var(--quiet)]">
                <span aria-hidden="true" className="mt-[7px] size-1 shrink-0 rounded-full bg-[var(--accent)]" />
                <span>Working…</span>
              </li>
            )}
          </ol>
        ) : (
          <div className="flex h-full flex-col justify-center px-5 py-8">
            <h3 className="text-sm font-medium text-[var(--text)]">No run yet</h3>
            <p className="mt-1 text-[13px] leading-6 text-[var(--muted)]">
              {running
                ? "Starting…"
                : "Press Run now on the Taste settings page. A run reads your own messages and records what it learned here, step by step."}
            </p>
          </div>
        )}

        {/* The finished run, once there is one, is the part worth keeping: its
            totals and the profile as it stood at the end. While it is running the
            live lines are the whole story. */}
        {!running && lastRun && (
          <div className="border-t border-[var(--line)]">
            <TasteRunReport run={lastRun} />
          </div>
        )}
      </div>

      {error && (
        <p role="alert" className="shrink-0 border-t border-[var(--line)] px-4 py-2 text-[11px] text-[var(--danger)]">
          {error}
        </p>
      )}
    </div>
  );
}
