import { ChevronRight, Edit, Eye, FileCode2, FileText, Folder, Trash2 } from "lucide-react";
import type { TreeEntry } from "../../../features/rightPanel/useFileTree";
import { useEffect, useState } from "react";

type FileTreeRowProps = {
  entry: TreeEntry;
  depth: number;
  isExpanded: (path: string) => boolean;
  childrenOf: (path: string) => TreeEntry[] | undefined;
  loadingPath: string | null;
  onToggle: (path: string) => void;
  onOpenFile: (entry: TreeEntry) => void;
  onNotify?: (tone: "success" | "error", message: string) => void;
  onRename: (path: string, newName: string) => Promise<void>;
  onDelete: (path: string) => Promise<void>;
};

/** Icon by file extension, falling back to a generic page for anything unknown. */
function FileIcon({ name }: { name: string }) {
  const extension = name.slice(name.lastIndexOf(".") + 1).toLowerCase();
  const code = ["ts", "tsx", "js", "jsx", "rs", "py", "json", "css", "html", "toml", "sql", "sh", "yml", "yaml", "go", "java", "cs"];
  return code.includes(extension)
    ? <FileCode2 size={14} className="shrink-0 text-[var(--muted)]" />
    : <FileText size={14} className="shrink-0 text-[var(--muted)]" />;
}

export default function FileTreeRow({ entry, depth, isExpanded, childrenOf, loadingPath, onToggle, onOpenFile, onNotify, onRename, onDelete }: FileTreeRowProps) {
  const expanded = entry.isDir && isExpanded(entry.path);
  const children = expanded ? childrenOf(entry.path) : undefined;
  const isLoading = loadingPath === entry.path;
  const [anchorPoint, setAnchorPoint] = useState<{x: number, y: number} | null>(null);

  // Close context menu when clicking outside or pressing Escape
  useEffect(() => {
    const handleClickOutside = (event: MouseEvent) => {
      if (!anchorPoint) return;
      const target = event.target as HTMLElement;
      // Don't close if clicking on the menu itself or its children
      if (target.closest('[data-context-menu]')) return;
      setAnchorPoint(null);
    };

    const handleEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape" && anchorPoint) {
        setAnchorPoint(null);
      }
    };

    document.addEventListener("mousedown", handleClickOutside);
    document.addEventListener("keydown", handleEscape);
    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
      document.removeEventListener("keydown", handleEscape);
    };
  }, [anchorPoint]);

  const handleView = () => {
    onOpenFile(entry);
  };

  const handleRename = async () => {
    const newName = window.prompt("Enter new name:", entry.name);
    if (newName === null) return;
    if (!newName.trim()) {
      onNotify?.("error", "Name cannot be empty");
      return;
    }
    if (newName === entry.name) return;
    try {
      await onRename(entry.path, newName);
    } catch (error) {
      onNotify?.("error", `Failed to rename: ${(error as Error).message}`);
    }
    setAnchorPoint(null);
  };

  const handleDelete = async () => {
    const confirmed = window.confirm(`Delete "${entry.name}"?`);
    if (!confirmed) return;
    try {
      await onDelete(entry.path);
    } catch (error) {
      onNotify?.("error", `Failed to delete: ${(error as Error).message}`);
    }
    setAnchorPoint(null);
  };

  return (
    <>
      <div
        // Depth as padding rather than nested wrappers: a wrapper per level would
        // make the indent a layout change on every expand and would put a border
        // or background on the level rather than the row.
        style={{ paddingLeft: `${depth * 12 + 8}px` }}
        className="group flex min-h-7 w-full items-center gap-1 rounded-md pr-2 text-[13px] text-[var(--text)] transition-colors hover:bg-[var(--raised)]"
        onContextMenu={(event) => {
          event.preventDefault(); // Prevent browser context menu
          setAnchorPoint({ x: event.clientX, y: event.clientY });
        }}
      >
        {entry.isDir ? (
          <button
            type="button"
            onClick={() => onToggle(entry.path)}
            aria-expanded={expanded}
            aria-label={expanded ? `Collapse ${entry.name}` : `Expand ${entry.name}`}
            // The caret is `w-4` even on a file, where it holds an invisible
            // placeholder, so a file's name lines up with a folder's name beside
            // it instead of stepping left.
            className="grid size-4 shrink-0 place-items-center text-[var(--quiet)]"
          >
            <ChevronRight size={13} className={`transition-transform ${expanded ? "rotate-90" : ""}`} />
          </button>
        ) : (
          <span className="size-4 shrink-0" />
        )}

        <button
          type="button"
          onClick={() => (entry.isDir ? onToggle(entry.path) : onOpenFile(entry))}
          className="flex min-w-0 flex-1 items-center gap-1.5 py-1 text-left focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-[var(--accent)]"
        >
          {entry.isDir
            ? <Folder size={14} className="shrink-0 text-[var(--muted)]" />
            : <FileIcon name={entry.name} />}
          <span className="truncate">{entry.name}</span>
        </button>

        {/* A spinner on the row being fetched, rather than a panel-wide one: it
            says which folder is loading, which is the only useful fact. */}
        {isLoading && <span className="size-3 shrink-0 animate-spin rounded-full border-2 border-[var(--line)] border-t-[var(--accent)]" />}
      </div>

      {/* Children render below their folder only while it is open. A collapsed
          subtree is not drawn at all, so a deep tree costs only what is visible. */}
      {expanded && children?.map((child) => (
        <FileTreeRow
          key={child.path}
          entry={child}
          depth={depth + 1}
          isExpanded={isExpanded}
          childrenOf={childrenOf}
          loadingPath={loadingPath}
          onToggle={onToggle}
          onOpenFile={onOpenFile}
          onNotify={onNotify}
          onRename={onRename}
          onDelete={onDelete}
        />
      ))}

      {/* Context menu */}
      {anchorPoint && (
        <div
          data-context-menu
          className="fixed z-50"
          style={{
            left: Math.min(anchorPoint.x, window.innerWidth - 150), // prevent off-screen right
            top: Math.min(anchorPoint.y, window.innerHeight - 100), // prevent off-screen bottom
          }}
        >
          <div className="relative rounded-md shadow-lg bg-[var(--page)] ring-1 ring-black ring-opacity-5 p-1">
            {/* View button */}
            <div
              className="flex items-center px-2 py-1 text-sm text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)] cursor-default"
              onClick={handleView}
            >
              <Eye size={14} className="shrink-0 mr-2" />
              <span>View</span>
            </div>
            
            {/* Rename button */}
            <div
              className="flex items-center px-2 py-1 text-sm text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)] cursor-default"
              onClick={handleRename}
            >
              <Edit size={14} className="shrink-0 mr-2" />
              <span>Rename</span>
            </div>
            
            {/* Delete button */}
            <div
              className="flex items-center px-2 py-1 text-sm text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)] cursor-default"
              onClick={handleDelete}
            >
              <Trash2 size={14} className="shrink-0 mr-2" />
              <span>Delete</span>
            </div>
          </div>
        </div>
      )}
    </>
  );
}