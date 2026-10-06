import { X } from "lucide-react";

/**
 * The browser's own tab strip: one row per page, above the address bar.
 *
 * Chrome-shaped but cheaper: a label per tab (host or "New tab") with its close
 * inside, plus a `+` for a fresh tab. Tabs live in Rust -- the strip renders
 * `BrowserState.tabs` and asks to switch, it never invents one.
 */

export type BrowserTabEntry = { id: number; url: string };

/** Short label for a tab: host of the URL, or "New tab" before it loads. */
export function tabLabel(url: string): string {
  if (!url) return "New tab";
  try {
    const parsed = new URL(url);
    if (parsed.protocol === "file:") {
      const parts = parsed.pathname.split("/").filter(Boolean);
      return decodeURIComponent(parts[parts.length - 1] ?? "File");
    }
    return parsed.hostname || url;
  } catch {
    return url.length > 18 ? `${url.slice(0, 18)}…` : url;
  }
}

type BrowserTabsProps = {
  tabs: BrowserTabEntry[];
  activeTab: number | null;
  onSelect: (id: number) => void;
  onClose: (id: number) => void;
  onNew: () => void;
};

export default function BrowserTabs({ tabs, activeTab, onSelect, onClose, onNew }: BrowserTabsProps) {
  if (tabs.length === 0) return null;
  return (
    <div role="tablist" aria-label="Browser tabs" className="flex min-h-9 shrink-0 items-center gap-1 overflow-x-auto border-b border-[var(--line)] px-2">
      {tabs.map((tab) => {
        const active = tab.id === activeTab;
        const label = tabLabel(tab.url);
        return (
          <button
            key={tab.id}
            type="button"
            role="tab"
            aria-selected={active}
            title={tab.url || "New tab"}
            onClick={() => onSelect(tab.id)}
            className={`group/btab flex min-w-0 max-w-40 shrink-0 items-center gap-1 rounded-md px-2 py-1 text-xs transition-colors focus-visible:outline-2 focus-visible:outline-[var(--accent)] ${active ? "bg-[var(--raised)] font-medium text-[var(--text)]" : "text-[var(--muted)] hover:text-[var(--text)]"}`}
          >
            <span className="min-w-0 flex-1 truncate">{label}</span>
            {/* Inside the tab, shown on hover or focus: an always-visible mark on
                every tab is noise, and a separate outside button breaks the tab
                shape. Keyboard keeps it reachable via focus-within. */}
            <span
              role="button"
              tabIndex={0}
              aria-label={`Close ${label}`}
              title={`Close ${label}`}
              onClick={(event) => { event.stopPropagation(); onClose(tab.id); }}
              onKeyDown={(event) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); event.stopPropagation(); onClose(tab.id); } }}
              className="grid size-4 shrink-0 place-items-center rounded text-[var(--quiet)] opacity-0 transition-opacity hover:bg-[var(--line)] hover:text-[var(--text)] focus-visible:opacity-100 focus-visible:outline-2 focus-visible:outline-[var(--accent)] group-hover/btab:opacity-100 group-focus-within/btab:opacity-100"
            >
              <X size={10} />
            </span>
          </button>
        );
      })}
      {/* Chrome's `+`: a fresh tab drawing the empty state, not a blank page. */}
      <button
        type="button"
        onClick={onNew}
        aria-label="New tab"
        title="New tab"
        className="grid size-6 shrink-0 place-items-center rounded text-[var(--muted)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
      >
        <span aria-hidden="true" className="text-sm leading-none">+</span>
      </button>
    </div>
  );
}
