/** View switcher for the session overview.
    Underlined rather than pill-shaped: the strip is chrome that separates from
    the record below, not a filter over it, so it reads correctly with a mouse
    and with a keyboard. */

export const SESSION_OVERVIEW_TABS = ["Overview", "Statistics", "Advanced"] as const;

export type SessionOverviewTab = (typeof SESSION_OVERVIEW_TABS)[number];

type TabStripProps = {
  value: SessionOverviewTab;
  onChange: (next: SessionOverviewTab) => void;
};
export default function TabStrip({ value, onChange }: TabStripProps) {
  return (
    <div role="tablist" aria-label="Session detail view" className="flex shrink-0 gap-1 border-b border-[var(--line)] px-3">
      {SESSION_OVERVIEW_TABS.map((tab) => {
        const selected = tab === value;
        return (
          <button
            key={tab}
            type="button"
            role="tab"
            id={`session-tab-${tab.toLowerCase()}`}
            aria-selected={selected}
            aria-controls="session-tab-panel"
            tabIndex={selected ? 0 : -1}
            onClick={() => onChange(tab)}
            className={`relative min-h-10 px-2.5 text-[13px] transition-colors focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-[var(--accent)] ${selected ? "font-medium text-[var(--text)]" : "text-[var(--muted)] hover:text-[var(--text)]"}`}
          >
            {tab}
            {/* Underline sits on the button's own bottom edge, so it reads as
                part of the strip rather than a floating rule. */}
            {selected && <span aria-hidden="true" className="absolute inset-x-2.5 -bottom-px h-0.5 rounded-full bg-[var(--accent)]" />}
          </button>
        );
      })}
    </div>
  );
}
