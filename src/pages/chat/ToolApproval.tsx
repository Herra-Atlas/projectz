import { useEffect, useRef } from "react";
import ToolIcon from "../../components/ToolIcon";

/** A tool call waiting on the user. */
export type PendingApproval = {
  /** Ties the dialog to the run, so two sessions can each have one open. */
  runId: string;
  approvalId: string;
  tool: string;
  /** Empty for a tool that is not a shell. */
  command: string;
  arguments: Record<string, unknown>;
};

type ToolApprovalProps = {
  request: PendingApproval;
  onAnswer: (approvalId: string, allow: boolean) => void;
  onCancel: () => void;
};

/**
 * Asks the user to allow one tool call.
 *
 * **A bar floating above the composer, not a dialog over the app.** It used to
 * be a modal: the whole window dimmed and the conversation disappeared behind a
 * card asking about one shell command. That is the loudest thing the interface
 * can do over work in progress, and the thing it interrupted -- reading what the
 * agent had done so far -- is exactly what is needed to answer the question. The
 * command refers to files visible in the transcript, so the prompt sits just
 * above the composer with that transcript still in view.
 *
 * Above the composer rather than inside it. Inside, it read as one more
 * attachment on a draft the user was still editing, and it grew the composer's
 * card to accommodate a question that was not about the draft. Above, the
 * composer is untouched and the prompt is plainly a separate thing waiting for
 * an answer. The gap is one `mb`: close enough to read the two as a pair, far
 * enough that they are not one control.
 *
 * **Styled as a tool row.** Same 12px type, same `--muted` label, same icon and
 * same monospace command as the rows in the activity panel, because it *is* one:
 * the same call, shown a moment earlier while it waits. Only the accent-tinted
 * edge and the shadow mark it as wanting an answer.
 *
 * Still blocking, and still with no way to dismiss it unanswered: Escape declines
 * rather than leaving the run parked on a prompt nobody can reach.
 */
export default function ToolApproval({ request, onAnswer, onCancel }: ToolApprovalProps) {
  const acceptRef = useRef<HTMLButtonElement>(null);

  // Focus the safe choice, so a stray Enter cannot grant the unsafe one.
  useEffect(() => { acceptRef.current?.focus(); }, []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onAnswer(request.approvalId, false);
        onCancel();
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [onAnswer, onCancel, request.approvalId]);

  // Shown verbatim rather than summarised. A user approving `cargo build` has to
  // be looking at `cargo build`, and a shortened rendering of a long command is
  // exactly the kind of thing that hides an appended `&& rm`.
  const command = request.command.trim();

  return (
    <div
      role="alertdialog"
      aria-modal="false"
      aria-labelledby="tool-approval-title"
      // Its own surface, because it no longer sits inside the composer's card.
      // The same tokens the composer uses, so the two read as the same family of
      // surface -- one raised above the other rather than two unrelated objects.
      // The accent-tinted edge is the only signal that this one wants an answer,
      // which is why it is on the border and not on the buttons or the text.
      className="flex flex-wrap items-center gap-x-2 gap-y-1.5 rounded-lg border border-[color-mix(in_srgb,var(--accent)_30%,var(--line))] bg-[var(--panel)] px-3 py-2 shadow-[0_10px_28px_rgba(0,0,0,0.22)]"
    >
      <ToolIcon tool={request.tool} />
      <span id="tool-approval-title" className="shrink-0 text-xs text-[var(--muted)]">
        Allow
      </span>
      {/* The command in `--text` rather than as a heading: this is a row, and the
          call it is about is named by the command beside it. Monospaced, because
          a command is code and proportional spacing makes `rm` and `rn` harder to
          tell apart at a glance. Truncates rather than wrapping, so a long command
          cannot push the two buttons off the row and out of reach. */}
      <span className="min-w-0 flex-1 truncate font-mono text-xs text-[var(--text)]">
        {command || request.tool}
      </span>

      {/* `type="button"` on both, because this still sits inside the composer's
          form and a bare button here would submit the draft as the answer. */}
      <button
        type="button"
        onClick={() => { onAnswer(request.approvalId, false); onCancel(); }}
        className="inline-flex h-7 shrink-0 items-center rounded-md border border-[var(--line)] px-2.5 text-[11px] text-[var(--muted)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
      >
        Decline
      </button>
      <button
        ref={acceptRef}
        type="button"
        onClick={() => { onAnswer(request.approvalId, true); onCancel(); }}
        className="inline-flex h-7 shrink-0 items-center rounded-md bg-[var(--accent)] px-2.5 text-[11px] font-medium text-[var(--accent-ink)] transition-opacity hover:opacity-90 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
      >
        Accept
      </button>
    </div>
  );
}