import { Bot, FileText, Globe2, Terminal, X, type LucideIcon } from "lucide-react";
import type { PanelViewId } from "../../features/rightPanel/usePanelViews";

/**
 * The strip of open views along the top of the panel.
 *
 * Each view is one chip: an icon, a label, and a close that appears on hover or
 * keyboard focus. The icon is what lets a glance pick out Files from Terminal in
 * a narrow strip, and it is the same mark the view picker uses, so a view looks
 * the same whether it is being chosen or already open.
 *
 * The active chip is tinted with the accent, matching the active entry in the
 * sidebar's nav -- the panel is one more place in the app that says "this one is
 * current", so it says it the same way. The `+` at the end reopens the view
 * picker (the "Nothing open" page), so a closed view is one click away.
 */

const TABS: Record<PanelViewId, { label: string; icon: LucideIcon }> = {
  files: { label: "Files", icon: FileText },
  terminal: { label: "Terminal", icon: Terminal },
  browser: { label: "Browser", icon: Globe2 },
  subagents: { label: "Sub agents", icon: Bot },
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
        const { label, icon: Icon } = TABS[view];
        return (
          <button
            key={view}
            type="button"
            role="tab"
            aria-selected={active}
            title={label}
            onClick={() => onSelect(view)}
            className={`group/vtab flex h-7 shrink-0 items-center gap-1.5 rounded-md py-1 pl-2 pr-1 text-xs font-medium transition-colors focus-visible:outline-2 focus-visible:outline-[var(--accent)] ${active ? "bg-[color-mix(in_srgb,var(--accent)_14%,transparent)] text-[var(--accent)]" : "text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]"}`}
          >
            <Icon size={13} className="shrink-0" />
            <span className="truncate">{label}</span>
            {!pinned && (
              <span
                role="button"
                tabIndex={0}
                aria-label={`Close ${label}`}
                title={`Close ${label}`}
                onClick={(event) => { event.stopPropagation(); onClose(view); }}
                onKeyDown={(event) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); event.stopPropagation(); onClose(view); } }}
                className={`grid size-4 shrink-0 place-items-center rounded transition-opacity hover:bg-[var(--line)] hover:text-[var(--text)] focus-visible:opacity-100 focus-visible:outline-2 focus-visible:outline-[var(--accent)] group-hover/vtab:opacity-100 group-focus-within/vtab:opacity-100 ${active ? "opacity-60" : "opacity-0"}`}
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
        className="grid size-7 shrink-0 place-items-center rounded-md text-[var(--muted)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
      >
        <span aria-hidden="true" className="text-base leading-none">+</span>
      </button>
    </div>
  );
}
