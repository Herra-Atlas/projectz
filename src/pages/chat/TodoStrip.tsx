import { useState } from "react";
import { ChevronRight, ListChecks } from "lucide-react";

/** One step on the agent's checklist. */
export type TodoEntry = {
  text: string;
  status: "pending" | "in_progress" | "completed";
};

/** A mark per state, so the list is readable at a glance without colour. */
const MARK: Record<TodoEntry["status"], string> = {
  pending: "○",
  in_progress: "◐",
  completed: "✓",
};

/**
 * The agent's checklist, above the composer.
 *
 * **Collapsed to one line by default.** The point of showing it is that a long
 * task has a shape the user can check at a glance -- "3 of 7 done, currently on
 * the parser" -- and a full list above the composer would push the actual
 * conversation down for the whole of a long task. The line states the progress
 * and the step in hand; opening it shows the rest.
 *
 * **A strip rather than a panel.** This is a per-turn fact about the reply in
 * progress, like the approval prompt it sits beside, not a page to be browsed.
 */
export default function TodoStrip({ items }: { items: TodoEntry[] }) {
  const [expanded, setExpanded] = useState(false);
  const done = items.filter((item) => item.status === "completed").length;
  // The step in hand, or the next one not yet done, so the line always names the
  // current focus rather than the first item.
  const current = items.find((item) => item.status === "in_progress") ?? items.find((item) => item.status !== "completed");

  return (
    <div className="mb-2 text-[12px] leading-6 text-[var(--muted)]">
      <button
        type="button"
        onClick={() => setExpanded((value) => !value)}
        aria-expanded={expanded}
        className="flex w-full min-w-0 items-center gap-1.5 rounded py-0.5 text-left hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
      >
        <ListChecks size={12} className="shrink-0 text-[var(--quiet)]" />
        <span className="shrink-0 font-mono text-[11px] tabular-nums text-[var(--quiet)]">
          {done}/{items.length}
        </span>
        {current && !expanded && (
          <span className="min-w-0 truncate">{current.text}</span>
        )}
        <ChevronRight
          size={12}
          aria-hidden
          className={`shrink-0 transition-transform ${expanded ? "rotate-90" : ""}`}
        />
      </button>

      {expanded && (
        <ul className="ml-[18px] mt-0.5 space-y-0.5 border-l border-[var(--line)] pl-3">
          {items.map((item, index) => (
            <li key={`${index}-${item.text}`} className="flex min-w-0 items-start gap-1.5 text-[11px]">
              <span
                aria-hidden
                className={`shrink-0 ${item.status === "completed" ? "text-[var(--accent)]" : "text-[var(--quiet)]"}`}
              >
                {MARK[item.status]}
              </span>
              <span className={`min-w-0 ${item.status === "completed" ? "text-[var(--quiet)] line-through" : "text-[var(--muted)]"}`}>
                {item.text}
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
