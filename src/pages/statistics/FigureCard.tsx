import type { ReactNode } from "react";
import { ArrowDownRight, ArrowUpRight, Minus } from "lucide-react";

/** How a figure moved against the period before it.
 *
 * `label` arrives already formatted — `16.2%` or `4.2 pts` — because the card
 * does not know whether the number above it is a count or a rate. `direction`
 * decides only the arrow and the colour. */
export type FigureChange = {
  direction: "up" | "down" | "flat";
  label: string;
};

/** One figure in the header row.
 *
 * Label on top, the figure large beneath it, then a line saying what it did
 * against the period before. The label is quiet and the number is loud, which
 * puts the number where the eye lands; the footer is the qualifier, so it is
 * read only once the figure has been. The footer keeps its height even when it
 * is empty, so every card in the row is the same height and the numbers line
 * up. */
type FigureCardProps = {
  label: string;
  value: string;
  change?: FigureChange | null;
  note?: string;
};

export function FigureCard({ label, value, change, note }: FigureCardProps) {
  return (
    <div className="min-w-0 bg-[var(--panel)] px-4 py-3.5">
      <p className="truncate text-xs text-[var(--muted)]" title={label}>
        {label}
      </p>
      <p
        className="mt-1.5 truncate text-[26px] font-semibold leading-none tracking-tight tabular-nums text-[var(--text)]"
        title={value}
      >
        {value}
      </p>
      <div className="mt-2.5 flex min-h-4 min-w-0 items-center gap-1.5 text-[11px] text-[var(--quiet)]">
        {change && <ChangePill change={change} />}
        {note && (
          <span className="truncate" title={note}>
            {note}
          </span>
        )}
      </div>
    </div>
  );
}

/** The change against the previous period, as a tinted pill with an arrow.
 *
 * A rise takes `--added` and a fall `--removed` — the same pair the diff view
 * uses, so the two directions read as one figure of speech across the app. A
 * flat figure takes the quiet colours, because no change is not a win. The tint
 * is mixed from the token rather than being a second colour, so it follows the
 * theme with the text beside it. */
function ChangePill({ change }: { change: FigureChange }) {
  const tone =
    change.direction === "up"
      ? { color: "var(--added)", background: "color-mix(in srgb, var(--added) 15%, transparent)" }
      : change.direction === "down"
        ? { color: "var(--removed)", background: "color-mix(in srgb, var(--removed) 15%, transparent)" }
        : { color: "var(--quiet)", background: "var(--raised)" };
  const Icon = change.direction === "up" ? ArrowUpRight : change.direction === "down" ? ArrowDownRight : Minus;

  return (
    <span
      className="inline-flex shrink-0 items-center gap-0.5 rounded-full py-0.5 pl-1 pr-1.5 text-[11px] font-medium tabular-nums"
      style={tone}
    >
      <Icon size={11} strokeWidth={2.5} aria-hidden />
      {change.label}
    </span>
  );
}

/** A panel for a chart or table. Hairline border, no shadow, matching the
    settings pages so the page reads as one system.
 *
 * `title` and `tabs` are alternatives rather than a pair: a panel whose views
 * are all named — the token chart's three — needs no heading above them, while
 * one whose tabs are short labels — the models' `All`, `Local`, a provider —
 * still needs the noun they belong to. `action` sits at the right, so a loading
 * note does not stand among the buttons. */
type PanelProps = { title?: string; tabs?: ReactNode; children: ReactNode; action?: ReactNode };

export function Panel({ title, tabs, children, action }: PanelProps) {
  return (
    <section className="flex min-w-0 flex-col rounded-lg border border-[var(--line)] bg-[var(--panel)]">
      <div className="flex min-h-[38px] items-center gap-3 border-b border-[var(--line)] px-4 py-1.5">
        {title && <h2 className="shrink-0 truncate text-[13px] font-medium text-[var(--text)]">{title}</h2>}
        {tabs && <div className="min-w-0 flex-1">{tabs}</div>}
        {action && <div className="ml-auto flex shrink-0 items-center">{action}</div>}
      </div>
      {children}
    </section>
  );
}
