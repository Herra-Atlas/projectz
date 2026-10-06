import type { ReactNode } from "react";

/** Shared presentational primitives for the session overview tabs.
    The tabs stay free of layout maths so the figures remain the only thing that
    changes between them. */

type StatProps = {
  label: string;
  value: string;
  /** Secondary line under the figure. Says where the number came from, so a
      derived value is never mistaken for a measured one. */
  hint?: string;
};

export function Stat({ label, value, hint }: StatProps) {
  return (
    <div className="min-w-0">
      <p className="text-[11px] text-[var(--quiet)]">{label}</p>
      <p className="mt-1 truncate text-lg font-medium tabular-nums text-[var(--text)]" title={value}>{value}</p>
      {hint && <p className="mt-0.5 truncate text-[11px] text-[var(--quiet)]" title={hint}>{hint}</p>}
    </div>
  );
}

type GroupProps = { icon: ReactNode; title: string; children: ReactNode };

/** A titled band of figures, separated by a hairline rather than a card, so the
    groups read as one continuous record instead of stacked panels. */
export function Group({ icon, title, children }: GroupProps) {
  return (
    <section className="border-t border-[var(--line)] px-5 py-4 first:border-t-0">
      <h3 className="flex items-center gap-2 text-[11px] font-medium text-[var(--quiet)]">
        <span className="text-[var(--muted)]">{icon}</span>
        {title}
      </h3>
      <div className="mt-3">{children}</div>
    </section>
  );
}

/** The grid every group of small figures uses. */
export function StatGrid({ children, columns = 3 }: { children: ReactNode; columns?: 2 | 3 }) {
  return (
    <div className={`grid gap-x-6 gap-y-4 ${columns === 2 ? "grid-cols-2" : "grid-cols-2 sm:grid-cols-3"}`}>
      {children}
    </div>
  );
}
