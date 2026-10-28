/** One view a panel can show, as a button in its header. */
export type PanelTab = {
  label: string;
  active: boolean;
  onSelect: () => void;
};

/**
 * The views of a panel, drawn as text buttons beside its title.
 *
 * A panel with more than one view used to page through them with arrows and a
 * `1/3` position, which named a count rather than the things being counted: a
 * reader had to press through the views to find the one they wanted. Every view
 * is named here and the current one is filled, so switching is one press and the
 * alternatives are visible before it.
 *
 * The row wraps instead of trimming or scrolling sideways. The Models panel has
 * one tab per provider, and a label cut to an ellipsis names nothing; a row that
 * scrolls sideways would hide views behind a gesture nobody performs over a
 * header. Wrapping keeps every view named and reachable, at the cost of a
 * second line when there are many providers.
 *
 * A single view draws no control at all, for the same reason the old arrows did
 * not: a button that cannot change anything is a control that lies about being
 * one.
 */
export function PanelTabs({ tabs, what }: { tabs: PanelTab[]; what: string }) {
  if (tabs.length <= 1) return null;

  return (
    <div
      role="group"
      aria-label={what}
      className="flex min-w-0 flex-wrap items-center gap-0.5"
    >
      {tabs.map((tab) => (
        <button
          key={tab.label}
          type="button"
          aria-pressed={tab.active}
          onClick={tab.onSelect}
          className={`shrink-0 whitespace-nowrap rounded-md px-2 py-1 text-xs transition-colors focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)] ${
            tab.active
              ? "bg-[var(--raised)] font-medium text-[var(--text)]"
              : "text-[var(--quiet)] hover:text-[var(--text)]"
          }`}
        >
          {tab.label}
        </button>
      ))}
    </div>
  );
}
