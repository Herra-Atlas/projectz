import { useEffect, useRef } from "react";
import { Check, FolderOpen, X } from "lucide-react";
import { workspaceLabel, type WorkspaceState } from "../features/workspace/useWorkspaces";

type WorkspacePickerProps = {
  workspaces: WorkspaceState;
  /** Opens the system chooser and adds whatever comes back. */
  onOpen: () => void;
  onSelect: (path: string) => void;
  onClose: (path: string) => void;
  /** Closes the menu without choosing anything. */
  onDismiss: () => void;
};

/**
 * The open workspaces, with the current one marked, and one way to add another.
 *
 * Only **Open folder** is offered. VS Code's menu also carries "New project" and
 * "Don't work in a project", and both are meaningless here: ProjectZ has no
 * concept of a project file, and it does work in a folder. Offering them would be
 * two rows that lead nowhere.
 *
 * Deliberately narrow. Every row is a folder name, a tick, and a close button that
 * appears only on hover -- no drive icons, no path preview, no paragraph
 * explaining what a workspace is. The full path is on hover for anyone who needs
 * to tell two same-named folders apart, and a menu is not the place to be taught
 * in.
 */
export default function WorkspacePicker({
  workspaces,
  onOpen,
  onSelect,
  onClose,
  onDismiss,
}: WorkspacePickerProps) {
  const containerRef = useRef<HTMLDivElement>(null);

  // Dismissed by clicking away or pressing Escape, like every other menu in the
  // app. A menu with no way out but picking something would trap the user who
  // opened it by mistake.
  useEffect(() => {
    const dismissOutside = (event: PointerEvent) => {
      if (containerRef.current && !containerRef.current.contains(event.target as Node)) {
        onDismiss();
      }
    };
    const dismissEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") onDismiss();
    };
    document.addEventListener("pointerdown", dismissOutside);
    document.addEventListener("keydown", dismissEscape);
    return () => {
      document.removeEventListener("pointerdown", dismissOutside);
      document.removeEventListener("keydown", dismissEscape);
    };
  }, [onDismiss]);

  return (
    <div
      ref={containerRef}
      role="menu"
      aria-label="Workspaces"
      className="absolute left-0 top-full z-50 mt-1.5 w-64 overflow-hidden rounded-lg border border-[var(--line)] bg-[var(--rail)] p-1 shadow-xl"
    >
      {workspaces.paths.map((path) => {
        const isCurrent = path === workspaces.selected;
        return (
          <div key={path} className="group flex items-center">
            <button
              type="button"
              role="menuitemradio"
              aria-checked={isCurrent}
              onClick={() => onSelect(path)}
              title={path}
              className="flex min-h-9 min-w-0 flex-1 items-center gap-2 rounded-md px-2 text-left text-[13px] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)]"
            >
              {/* The tick keeps its slot whether or not it is drawn, so every row
                  stays the same width and the names line up. Only the current
                  folder is accented: the only decision this menu exists to make
                  is "which of these", and one accent answers it. */}
              <Check
                size={14}
                className={isCurrent ? "shrink-0 text-[var(--accent)]" : "shrink-0 opacity-0"}
              />
              <span className={`truncate ${isCurrent ? "text-[var(--text)]" : "text-[var(--muted)]"}`}>
                {workspaceLabel(path)}
              </span>
            </button>
            <button
              type="button"
              onClick={() => onClose(path)}
              aria-label={`Close ${workspaceLabel(path)}`}
              title="Close"
              className="grid size-7 shrink-0 place-items-center rounded-md text-[var(--quiet)] opacity-0 transition-opacity hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:opacity-100 focus-visible:outline-2 focus-visible:outline-[var(--accent)] group-hover:opacity-100"
            >
              <X size={13} />
            </button>
          </div>
        );
      })}

      <div className="my-1 border-t border-[var(--line)]" />
      <button
        type="button"
        role="menuitem"
        onClick={onOpen}
        className="flex min-h-9 w-full items-center gap-2 rounded-md px-2 text-left text-[13px] text-[var(--text)] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)]"
      >
        <FolderOpen size={14} className="shrink-0 text-[var(--muted)]" />
        Open folder…
      </button>
    </div>
  );
}