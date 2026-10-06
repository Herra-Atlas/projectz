import type { ReactNode } from "react";

/** One figure in the header row.
 *
 * The value and its label sit on one line — label first, figure after — so a
 * row of these reads as a compact list rather than a stack of cards. The label
 * is quiet and the figure is loud, which puts the number where the eye lands
 * without needing a second line to explain it. */
type FigureCardProps = {
  icon: ReactNode;
  label: string;
  value: string;
};

export function FigureCard({ icon, label, value }: FigureCardProps) {
  return (
    <div className="flex min-w-0 items-center gap-2.5 rounded-lg border border-[var(--line)] bg-[var(--panel)] px-3.5 py-3">
      <span className="shrink-0 text-[var(--quiet)]">{icon}</span>
      <p className="truncate text-xs text-[var(--muted)]">{label}</p>
      <span className="ml-auto shrink-0 text-base font-medium tabular-nums tracking-tight text-[var(--text)]" title={value}>
        {value}
      </span>
    </div>
  );
}

/** A titled panel for a chart or table. Hairline border, no shadow, matching
    the settings pages so the page reads as one system. */
type PanelProps = { title: string; children: ReactNode; action?: ReactNode };

export function Panel({ title, children, action }: PanelProps) {
  return (
    <section className="flex min-w-0 flex-col rounded-lg border border-[var(--line)] bg-[var(--panel)]">
      <div className="flex items-center justify-between gap-3 border-b border-[var(--line)] px-4 py-2.5">
        <h2 className="truncate text-[13px] font-medium text-[var(--text)]">{title}</h2>
        {action}
      </div>
      {children}
    </section>
  );
}
