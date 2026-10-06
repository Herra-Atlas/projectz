import { X } from "lucide-react";
import type { PanelViewId } from "../../features/rightPanel/usePanelViews";

/**
 * The strip of open views along the top of the panel.
 *
 * Each view is one merged tab: label with its close inside, shown on hover or
 * keyboard focus. A separate outside button broke the tab shape and doubled the
 * marks in a strip that is already small. The `+` at the end reopens the view
 * picker -- the "Nothing open" page -- so a closed view is one click away.
 */

const LABELS: Record<PanelViewId, string> = {
  files: "Files",
  terminal: "Terminal",
  browser: "Browser",
};

type PanelTabsProps = {
  openViews: PanelViewId[];
  activeView: PanelViewId | null;
  onSelect: (view: PanelViewId) => void;
  onClose: (view: PanelViewId) => void;
  /** Shows the view picker ("Nothing open") without closing anything. */
  onNew: () => void;
  /** Views whose close buttons should be hidden, e.g. terminal while server is running */
  pinnedViews?: PanelViewId[];
};

export default function PanelTabs({ openViews, activeView, onSelect, onClose, onNew, pinnedViews = [] }: PanelTabsProps) {
  return (
    <div role="tablist" aria-label="Open views" className="flex min-w-0 items-center gap-1 overflow-x-auto">
      {openViews.map((view) => {
        const active = view === activeView;
        const pinned = pinnedViews.includes(view);
        return (
          <button
            key={view}
            type="button"
            role="tab"
            aria-selected={active}
            onClick={() => onSelect(view)}
            className={`group/vtab flex shrink-0 items-center gap-1 rounded-md py-1 pl-2 pr-1 text-xs font-medium transition-colors focus-visible:outline-2 focus-visible:outline-[var(--accent)] ${active ? "bg-[var(--raised)] text-[var(--text)]" : "text-[var(--muted)] hover:text-[var(--text)]"}`}
          >
            <span>{LABELS[view]}</span>
            {!pinned && (
              <span
                role="button"
                tabIndex={0}
                aria-label={`Close ${LABELS[view]}`}
                title={`Close ${LABELS[view]}`}
                onClick={(event) => { event.stopPropagation(); onClose(view); }}
                onKeyDown={(event) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); event.stopPropagation(); onClose(view); } }}
                className="grid size-4 shrink-0 place-items-center rounded text-[var(--quiet)] opacity-0 transition-opacity hover:bg-[var(--line)] hover:text-[var(--text)] focus-visible:opacity-100 focus-visible:outline-2 focus-visible:outline-[var(--accent)] group-hover/vtab:opacity-100 group-focus-within/vtab:opacity-100"
              >
                <X size={10} />
              </span>
            )}
          </button>
        );
      })}
      <button
        type="button"
        onClick={onNew}
        aria-label="Open a view"
        title="Open a view"
        className="grid size-6 shrink-0 place-items-center rounded text-[var(--muted)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
      >
        <span aria-hidden="true" className="text-sm leading-none">+</span>
      </button>
    </div>
  );
}
