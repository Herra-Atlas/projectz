import { useEffect, useRef, useState } from "react";
import { Search } from "lucide-react";
import FileTreeRow from "./FileTreeRow";
import FilePreview from "./FilePreview";
import { useFileTree, type TreeEntry } from "../../../features/rightPanel/useFileTree";
import { workspaceLabel } from "../../../features/workspace/useWorkspaces";

/**
 * The Files view: the workspace as a tree, with a preview of whatever is clicked.
 *
 * Three states in one component rather than three files, because only one is ever
 * visible and they share the same tree state -- a preview that forgot to render
 * its "back" row would strand the user in a file with no way to the tree.
 * Everything expensive is in `useFileTree` and the preview is its own file.
 */

type FilesTabProps = {
  /** The open workspace path, or null when none is chosen. */
  workspace: string | null;
  /**
   * A file to show, set from outside -- currently only by clicking a changed file
   * in a reply.
   *
   * Passed in rather than reached for through a context because the two features
   * live in different trees: the click happens in the transcript, which knows
   * nothing about the panel, and the panel knows nothing about replies. The one
   * value they share is a workspace-relative path.
   *
   * Cleared once shown, so opening the tree afterwards does not jump straight
   * back to the last file someone clicked -- the path is a request, not a
   * selection.
   */
  requestedPath?: string | null;
  /** Called after the request has been consumed. */
  onRequestHandled: () => void;
  /**
   * Asked to show an HTML file in the Browser view.
   *
   * FilesTab owns no browser; RightPanel does. So the file hands the URL up and
   * the shell opens the view -- one hop, no context.
   */
  onPreviewInBrowser: (fileUrl: string) => void;
  /** Reported so a refused address reaches the app's one notification stack. */
  onNotify?: (tone: "success" | "error", message: string) => void;
};

export default function FilesTab({ workspace, requestedPath, onRequestHandled, onPreviewInBrowser, onNotify }: FilesTabProps) {
  const [preview, setPreview] = useState<TreeEntry | null>(null);
  const [query, setQuery] = useState("");
  const { root, error, loadingPath, isExpanded, childrenOf, toggle, refresh, rename, delete: deleteFile } = useFileTree();

  /**
   * Shows a file asked for from outside the panel.
   *
   * A synthetic entry rather than a lookup in the loaded tree: the file may sit
   * inside a folder nobody expanded, so it is very often absent from what the
   * tree has fetched. Only the name and path matter to the preview.
   *
   * The acknowledgement is read through a ref rather than depended on. `App`
   * passes an inline arrow, so its identity changes every render -- and depending
   * on it re-ran this effect on every parent render, re-setting the preview and
   * re-acknowledging forever.
   */
  const acknowledgeRef = useRef(onRequestHandled);
  acknowledgeRef.current = onRequestHandled;

  useEffect(() => {
    if (!requestedPath) return;
    const name = requestedPath.slice(requestedPath.lastIndexOf("/") + 1);
    setPreview({ name, path: requestedPath, isDir: false, hasChildren: false });
    acknowledgeRef.current();
  }, [requestedPath]);

  // Re-read from the root whenever the folder changes. Without this the tree
  // would keep showing the *previous* workspace's contents next to the new
  // folder's name in the header, which is worse than an empty panel.
  //
  // **Reset only on a genuine switch.** This effect and the request effect above
  // both fire on mount when the panel opens *because* of a request -- that is the
  // normal path for clicking a changed file. Running the reset unconditionally
  // set the preview and cleared it in the same commit, so the first click appeared
  // to do nothing and only the second worked. The sentinel is what tells the two
  // apart: it starts as a value `workspace` can never be, so the first render
  // always counts as "just mounted" rather than as "the folder is still null".
  const loadedWorkspaceRef = useRef<string | null | undefined>(undefined);

  useEffect(() => {
    const previous = loadedWorkspaceRef.current;
    loadedWorkspaceRef.current = workspace;
    if (previous !== undefined) {
      // A genuine switch: anything open against the old folder is stale, so the
      // preview and the filter go and the tree is read again for the new one.
      if (previous === workspace) return;
      setPreview(null);
      setQuery("");
    }
    // First run: read the tree, but leave a requested file alone. It is on its way
    // in from the click that opened this panel, and clearing it here is what made
    // the first click on a changed file look lost.
    void refresh();
  }, [workspace, refresh]);

  // A file opened under a folder that has since changed may no longer exist, so
  // returning to the tree is the only way to see what is there now.
  if (preview) {
    return <FilePreview entry={preview} workspace={workspace} onBack={() => setPreview(null)} onPreviewInBrowser={onPreviewInBrowser} onNotify={onNotify} />;
  }

  if (!workspace) {
    return (
      <div className="flex h-full flex-col items-center justify-center px-5 text-center">
        <p className="text-sm text-[var(--text)]">No folder open</p>
        <p className="mt-1 text-[13px] text-[var(--muted)]">Choose a folder from the header to browse its files.</p>
      </div>
    );
  }

  const filtered = query.trim()
    ? (root ?? []).filter((entry) => entry.name.toLowerCase().includes(query.trim().toLowerCase()))
    : root;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {/* Folder name and search, matching the screenshots: the name in accent so
          it reads as *which* folder, and the search on the same row because it
          filters the same list. */}
      <div className="flex min-h-10 shrink-0 items-center gap-2 border-b border-[var(--line)] px-3">
        <span className="min-w-0 flex-1 truncate text-[13px] font-medium text-[var(--accent)]" title={workspace}>{workspaceLabel(workspace)}</span>
        <Search size={13} className="shrink-0 text-[var(--quiet)]" />
        <input
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder="Filter"
          aria-label="Filter files"
          className="w-16 shrink-0 rounded bg-transparent text-right text-xs text-[var(--text)] outline-none placeholder:text-[var(--quiet)] focus:w-28 focus:text-left"
        />
      </div>

      <div
        className="min-h-0 flex-1 overflow-y-auto py-1"
      >
        {error && <p className="px-3 py-2 text-xs text-[var(--danger)]">{error}</p>}
        {filtered === null && !error && <p className="px-3 py-2 text-xs text-[var(--quiet)]">Reading…</p>}
        {filtered?.length === 0 && <p className="px-3 py-2 text-xs text-[var(--quiet)]">Nothing here.</p>}
        {filtered?.map((entry) => (
            <FileTreeRow
              key={entry.path}
              entry={entry}
              depth={0}
              isExpanded={isExpanded}
              childrenOf={childrenOf}
              loadingPath={loadingPath}
              onToggle={(path) => void toggle(path)}
              onOpenFile={setPreview}
              onNotify={onNotify}
              onRename={rename}
              onDelete={deleteFile}
            />
          ))}
      </div>
    </div>
  );
}
