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

/** Normalize a path to workspace-relative format (forward slashes, no leading ./ or absolute paths). */
export function normalizePath(path: string): string {
  // Strip Windows extended-length prefix \\?\ (4 chars: \, \, ?, \)
  if (path.startsWith("\\\\?\\")) {
    path = path.slice(4);
  }
  // Convert backslashes to forward slashes
  let normalized = path.replace(/\\/g, "/");
  // Remove leading ./ if present
  if (normalized.startsWith("./")) {
    normalized = normalized.slice(2);
  }
  return normalized;
}

/** The folder a workspace-relative path lives in. The empty string is the root. */
export function parentOf(path: string): string {
  const slash = path.lastIndexOf("/");
  return slash === -1 ? "" : path.slice(0, slash);
}

/**
 * The absolute, on-disk path of a workspace-relative entry.
 *
 * Joined with the separator the workspace root already uses, so the result is
 * what the OS would accept -- `\` on Windows, `/` elsewhere -- rather than a
 * web-style path the user has to translate before pasting it anywhere useful.
 * The relative side is always forward-slashed (that is how the tree carries it),
 * so it is normalised on the way in.
 */
export function absolutePath(root: string, relative: string): string {
  const separator = root.includes("\\") ? "\\" : "/";
  const base = root.replace(/[\\/]+$/, "");
  return `${base}${separator}${relative.split("/").join(separator)}`;
}

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
      if (relative === "") {
        setRoot(entries);
      }
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
   * Opens a folder without toggling it shut, fetching its children the first time.
   *
   * Used before creating inside a folder: the new row is drawn among that
   * folder's children, so the folder has to be open for it to be visible -- and
   * [`toggle`] cannot serve here, because on an already-open folder it closes it.
   */
  const expand = useCallback(async (relative: string) => {
    setExpanded((current) => (current.has(relative) ? current : new Set(current).add(relative)));
    if (!(relative in childrenByPath)) await load(relative);
  }, [childrenByPath, load]);

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

  /**
   * Renames a file or directory.
   *
   * Re-reads the item's own folder rather than the whole tree: a rename keeps the
   * item where it was, and `refresh` would collapse every open folder and drop
   * the cache just to show one name change.
   *
   * @param sourcePath The workspace-relative path of the item to rename.
   * @param newName The new name for the item (without path).
   */
  const rename = useCallback(async (sourcePath: string, newName: string) => {
    const normalizedPath = normalizePath(sourcePath);
    try {
      await invoke("panel_rename_file", { relative: normalizedPath, newName });
      await load(parentOf(normalizedPath));
    } catch (reason) {
      throw new Error(String(reason));
    }
  }, [load]);

  /**
   * Deletes a file or directory. Re-reads the item's folder, for the reason the
   * rename does.
   *
   * @param path The workspace-relative path of the item to delete.
   */
  const del = useCallback(async (path: string) => {
    const normalizedPath = normalizePath(path);
    try {
      await invoke("panel_delete_file", { relative: normalizedPath });
      await load(parentOf(normalizedPath));
    } catch (reason) {
      throw new Error(String(reason));
    }
  }, [load]);

  /**
   * Creates a file or folder inside `parent` and re-reads just that folder.
   *
   * The user opened the folder in order to put something in it, so the tree stays
   * exactly as it was and only the one listing is fetched again to reveal the new
   * row -- `refresh` would close every open folder to show it.
   *
   * @param parent The workspace-relative folder to create in. `""` is the root.
   * @param name The new name (no path).
   * @param isDir Whether to create a folder rather than an empty file.
   */
  const create = useCallback(async (parent: string, name: string, isDir: boolean) => {
    const target = normalizePath(parent);
    try {
      await invoke(isDir ? "panel_create_folder" : "panel_create_file", { relative: target, name });
      await load(target);
    } catch (reason) {
      throw new Error(String(reason));
    }
  }, [load]);

  /**
   * Moves a file or folder into another folder, then re-reads both folders.
   *
   * Two listings rather than one because a move is the only action that changes
   * *two* places at once: the row has to disappear from where it was and appear
   * where it landed. Re-reading the whole tree would do that too, but would also
   * close every folder the user had opened.
   *
   * @param path The workspace-relative path of the item being moved.
   * @param targetDir The workspace-relative folder to move it into. `""` is root.
   */
  const move = useCallback(async (path: string, targetDir: string) => {
    const source = normalizePath(path);
    const target = normalizePath(targetDir);
    try {
      await invoke("panel_move_file", { relative: source, target });
      const from = parentOf(source);
      if (from === target) { await load(target); return; }
      await load(from);
      await load(target);
    } catch (reason) {
      throw new Error(String(reason));
    }
  }, [load]);

  return { 
    root, 
    error, 
    loadingPath, 
    isExpanded, 
    childrenOf, 
    toggle, 
    expand,
    collapseAll, 
    refresh,
    create,
    move,
    rename,
    delete: del,
  };
}