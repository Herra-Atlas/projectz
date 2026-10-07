import { ChevronRight, FileCode2, FileText, Folder } from "lucide-react";
import type { MouseEvent } from "react";
import { CreateNameRow, default as NameInput } from "./NameInput";
import type { FileEdit } from "../../../features/rightPanel/fileEdit";
import type { TreeDrag } from "../../../features/rightPanel/fileTreeDrag";
import type { TreeEntry } from "../../../features/rightPanel/useFileTree";

type FileTreeRowProps = {
  entry: TreeEntry;
  depth: number;
  isExpanded: (path: string) => boolean;
  childrenOf: (path: string) => TreeEntry[] | undefined;
  loadingPath: string | null;
  onToggle: (path: string) => void;
  onOpenFile: (entry: TreeEntry) => void;
  /** Right-click, raised to the tab so one menu serves the whole tree. */
  onContextMenu: (entry: TreeEntry, event: MouseEvent) => void;
  /** The one in-progress rename or create, or null when nothing is being named. */
  edit: FileEdit | null;
  onCommitEdit: (name: string) => void;
  onCancelEdit: () => void;
  /** Drag state and the one handler that starts a drag, shared by every row. */
  drag: TreeDrag;
};

/** Icon by file extension, falling back to a generic page for anything unknown. */
function FileIcon({ name }: { name: string }) {
  const extension = name.slice(name.lastIndexOf(".") + 1).toLowerCase();
  const code = ["ts", "tsx", "js", "jsx", "rs", "py", "json", "css", "html", "toml", "sql", "sh", "yml", "yaml", "go", "java", "cs"];
  return code.includes(extension)
    ? <FileCode2 size={14} className="shrink-0 text-[var(--muted)]" />
    : <FileText size={14} className="shrink-0 text-[var(--muted)]" />;
}

/**
 * One row of the workspace tree, and (when open) its subtree.
 *
 * The row owns no menu of its own: a right-click is reported up to `FilesTab`,
 * which draws one shared menu at the pointer. A drag is the same story -- the row
 * marks itself with `data-entry-path` and reports the press; `FilesTab` decides
 * what the pointer is over and what a drop means.
 */
export default function FileTreeRow({ entry, depth, isExpanded, childrenOf, loadingPath, onToggle, onOpenFile, onContextMenu, edit, onCommitEdit, onCancelEdit, drag }: FileTreeRowProps) {
  const expanded = entry.isDir && isExpanded(entry.path);
  const children = expanded ? childrenOf(entry.path) : undefined;
  const isLoading = loadingPath === entry.path;
  const renaming = edit?.kind === "rename" && edit.path === entry.path;
  // A create lands as the first child of its parent, so the row that *is* that
  // parent draws the field -- which is also why the parent has to be expanded.
  const creatingHere = edit?.kind === "create" && edit.parent === entry.path;
  const dragging = drag.source === entry.path;
  // A folder lights up while a draggable is over it, but only when the drop would
  // actually be accepted; an unreachable target must not look like a live one.
  const dropping = entry.isDir && drag.target === entry.path;

  const icon = entry.isDir
    ? <Folder size={14} className="shrink-0 text-[var(--muted)]" />
    : <FileIcon name={entry.name} />;

  return (
    <>
      <div
        // Depth as padding rather than nested wrappers: a wrapper per level would
        // make the indent a layout change on every expand and would put a border
        // or background on the level rather than the row.
        style={{ paddingLeft: `${depth * 12 + 8}px` }}
        // The drag reads these two marks back off the element under the pointer;
        // `data-entry-dir` is present only on a folder, which is the only row a
        // drop may land on.
        data-entry-path={entry.path}
        data-entry-dir={entry.isDir ? "true" : undefined}
        className={`group flex min-h-7 w-full items-center gap-1 rounded-md pr-2 text-[13px] text-[var(--text)] transition-[background-color,opacity] hover:bg-[var(--raised)] ${dragging ? "opacity-40" : ""} ${dropping ? "bg-[color-mix(in_srgb,var(--accent)_12%,transparent)] ring-1 ring-inset ring-[var(--accent)]" : ""}`}
        onContextMenu={(event) => onContextMenu(entry, event)}
        // Not draggable while its name is being typed: a press in the field is for
        // editing, and the hook also ignores presses that land in an input.
        onPointerDown={renaming ? undefined : (event) => drag.onPointerDown(entry, event)}
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

        {renaming ? (
          // The name button is replaced in place, so the row keeps its caret,
          // indent and metrics and only the label becomes editable.
          <div className="flex min-w-0 flex-1 items-center gap-1.5 py-1">
            {icon}
            <NameInput initialValue={edit.name} selectStem={!entry.isDir} onCommit={onCommitEdit} onCancel={onCancelEdit} />
          </div>
        ) : (
          <button
            type="button"
            onClick={() => (entry.isDir ? onToggle(entry.path) : onOpenFile(entry))}
            className="flex min-w-0 flex-1 items-center gap-1.5 py-1 text-left focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-[var(--accent)]"
          >
            {icon}
            <span className="truncate">{entry.name}</span>
          </button>
        )}

        {/* A spinner on the row being fetched, rather than a panel-wide one: it
            says which folder is loading, which is the only useful fact. */}
        {isLoading && <span className="size-3 shrink-0 animate-spin rounded-full border-2 border-[var(--line)] border-t-[var(--accent)]" />}
      </div>

      {/* The create field sits above the children so the new row appears where
          it will land, and only while the folder is open -- the same reason a
          collapsed subtree is not drawn at all. */}
      {expanded && creatingHere && (
        <CreateNameRow depth={depth + 1} isDir={edit.isDir} onCommit={onCommitEdit} onCancel={onCancelEdit} />
      )}

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
          onContextMenu={onContextMenu}
          edit={edit}
          onCommitEdit={onCommitEdit}
          onCancelEdit={onCancelEdit}
          drag={drag}
        />
      ))}
    </>
  );
}
