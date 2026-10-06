import { useCallback, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

/** One row of the workspace listing, as `panel_fs_list` returns it. */
export type TreeEntry = {
  name: string;
  /** Workspace-relative, forward slashes, and exactly what the next call takes. */
  path: string;
  isDir: boolean;
  /** Whether a folder has anything to expand into. Always false for a file. */
  hasChildren: boolean;
};

/** Listing one folder. `path` is `""` for the workspace root. */
export async function listFolder(relative: string): Promise<TreeEntry[]> {
  return invoke<TreeEntry[]>("panel_fs_list", { relative });
}

/**
 * The tree's own state: what is expanded, and what has been fetched.
 *
 * **Loaded folders are cached, not refetched.** `childrenByPath` holds one entry
 * per folder the user has opened, so collapsing and re-expanding is instant and
 * costs no IPC. It is also what keeps a folder's rows mounted in a stable order
 * rather than re-appearing from a pending state on every expand.
 *
 * The cache is keyed by path and never invalidated, which is honest here: a tree
 * is a snapshot of a folder, and a file the agent writes afterwards is picked up
 * when the panel reopens or the user collapses and re-expands that folder. Adding
 * invalidation now would mean deciding when to drop it, and a wrong answer either
 * discards rows the user is reading or hides ones that just appeared.
 */
export function useFileTree() {
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());
  const [childrenByPath, setChildrenByPath] = useState<Record<string, TreeEntry[]>>({});
  const [root, setRoot] = useState<TreeEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loadingPath, setLoadingPath] = useState<string | null>(null);

  const load = useCallback(async (relative: string) => {
    setLoadingPath(relative);
    setError(null);
    try {
      const entries = await listFolder(relative);
      setChildrenByPath((current) => ({ ...current, [relative]: entries }));
      // The root also gets its own slot, because the tree reads the root through
      // the same map. Without it the first level needs a second code path.
      if (relative === "") setRoot(entries);
      return entries;
    } catch (reason) {
      setError(String(reason));
      return null;
    } finally {
      setLoadingPath(null);
    }
  }, []);

  /** A folder's loaded children, or undefined when it has not been fetched. */
  const childrenOf = useCallback(
    (path: string) => childrenByPath[path],
    [childrenByPath],
  );

  const isExpanded = useCallback((relative: string) => expanded.has(relative), [expanded]);

  /**
   * Toggles a folder, fetching its children the first time.
   *
   * The fetch happens on *expand* rather than up front for the whole tree. A
   * workspace with thousands of files would otherwise be read in full before the
   * user clicked anything, and every row of it held in memory to be displayed
   * ten levels down.
   */
  const toggle = useCallback(async (relative: string) => {
    const open = expanded.has(relative);
    setExpanded((current) => {
      const next = new Set(current);
      if (open) next.delete(relative);
      else next.add(relative);
      return next;
    });
    if (!open && !(relative in childrenByPath)) await load(relative);
  }, [childrenByPath, expanded, load]);

  /** Collapses everything, which is what returning to the root should mean. */
  const collapseAll = useCallback(() => {
    setExpanded(new Set());
  }, []);

  /**
   * Discards the cache so the tree re-reads from disk.
   *
   * The `refreshKey` is bumped by the caller when it knows something changed --
   * a run finishing is the obvious case, since the agent writes files while the
   * tree is open and the user cannot see that.
   */
  const refresh = useCallback(async () => {
    setChildrenByPath({});
    setRoot(null);
    await load("");
  }, [load]);

  return { root, error, loadingPath, isExpanded, childrenOf, toggle, collapseAll, refresh };
}