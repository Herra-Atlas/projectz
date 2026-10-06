import { ChevronRight, FileCode2, FileText, Folder } from "lucide-react";
import type { TreeEntry } from "../../../features/rightPanel/useFileTree";

/**
 * One row of the file tree, and the rows under it.
 *
 * **Recursive rather than a flattened list.** A tree held flat has to be built
 * depth-first on every render and every row has to know its absolute depth to
 * draw its indent -- so a folder ten levels down needs the whole subtree above it
 * in memory just to be positioned. Recursing means a row needs its depth and its
 * entry, and the renderer stops at a collapsed folder without descending at all.
 *
 * **No state here.** Expanded-ness and the loaded children live in `useFileTree`
 * because they have to survive a collapse of an ancestor: unmounting the subtree
 * would lose every folder the user had opened inside it, so expanding a sibling
 * again would start from nothing. What this component owns is only whether its own
 * subtree is drawn, which it is told.
 */

type FileTreeRowProps = {
  entry: TreeEntry;
  depth: number;
  isExpanded: (path: string) => boolean;
  childrenOf: (path: string) => TreeEntry[] | undefined;
  loadingPath: string | null;
  onToggle: (path: string) => void;
  onOpenFile: (entry: TreeEntry) => void;
};

/** Icon by file extension, falling back to a generic page for anything unknown. */
function FileIcon({ name }: { name: string }) {
  const extension = name.slice(name.lastIndexOf(".") + 1).toLowerCase();
  const code = ["ts", "tsx", "js", "jsx", "rs", "py", "json", "css", "html", "toml", "sql", "sh", "yml", "yaml", "go", "java", "cs"];
  return code.includes(extension)
    ? <FileCode2 size={14} className="shrink-0 text-[var(--muted)]" />
    : <FileText size={14} className="shrink-0 text-[var(--muted)]" />;
}

export default function FileTreeRow({ entry, depth, isExpanded, childrenOf, loadingPath, onToggle, onOpenFile }: FileTreeRowProps) {
  const expanded = entry.isDir && isExpanded(entry.path);
  const children = expanded ? childrenOf(entry.path) : undefined;
  const isLoading = loadingPath === entry.path;

  return (
    <>
      <div
        // Depth as padding rather than nested wrappers: a wrapper per level would
        // make the indent a layout change on every expand and would put a border
        // or background on the level rather than the row.
        style={{ paddingLeft: `${depth * 12 + 8}px` }}
        className="group flex min-h-7 w-full items-center gap-1 rounded-md pr-2 text-[13px] text-[var(--text)] transition-colors hover:bg-[var(--raised)]"
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
        />
      ))}
    </>
  );
}