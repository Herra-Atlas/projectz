import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/** One row of the workspace listing, as `panel_fs_list` returns it. */
export type TreeEntry = {
  name: string;
  /** Workspace-relative, forward slashes, and exactly what the next call takes. */
  path: string;
  isDir: boolean;
  /** Whether a folder has anything to expand into. Always false for a file. */
  hasChildren: boolean;
};

/** The backend's per-item answer to acting on several rows at once. */
export type BatchSummary = { done: string[]; refused: string[] };

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

/** The Files panel's realtime event, emitted by `panel/watch.rs`. Payload: `{ folders }`. */
const WATCH_EVENT = "panel_fs_changed";

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

/**
 * Listing one folder. `path` is `""` for the root of the open tree.
 *
 * `skill` says which *root* the tree is showing, not which folder within it: any
 * slug selects the skills root (the folder every skill lives in), and `""` for
 * this argument selects the workspace. The path arguments are unchanged, so the
 * tree browses the skills root exactly as it browses a project -- which is the
 * point, since a skill is a folder and its neighbours are too.
 *
 * It stays on the same calls as the panel's other file operations
 * (`panel_create_folder`, `panel_rename_file`, and so on) because a folder the
 * user can see in the tree is a folder they can act on.
 */
export async function listFolder(relative: string, skill?: string | null): Promise<TreeEntry[]> {
  return invoke<TreeEntry[]>("panel_fs_list", { relative, skill: skill ?? null });
}

/**
 * The tree's own state: what is expanded, and what has been fetched.
 *
 * **Loaded folders are cached, not refetched.** `childrenByPath` holds one entry
 * per folder the user has opened, so collapsing and re-expanding is instant and
 * costs no IPC. It is also what keeps a folder's rows mounted in a stable order
 * rather than re-appearing from a pending state on every expand.
 *
 * The cache is not invalidated wholesale. What keeps it honest is the watcher:
 * the backend reports which folders' *listings* have changed, and every one this
 * tree has already drawn is re-fetched by name (see the `panel_fs_changed`
 * effect below). A folder it has never expanded costs nothing to leave cached,
 * because expanding it always fetches.
 *
 * **A skill root re-reads itself.** The workspace watcher is pointed at the open
 * workspace, so it says nothing about the skills root. An agent's `skill_write`
 * or a file saved by another program therefore has to be seen by this tree asking
 * again, which it does on the same event by re-reading what it has already drawn:
 * one call per open folder, only while such a root is showing, and none at all
 * otherwise. Folders that are closed are re-read on expand anyway, so they cost
 * nothing here.
 */
export function useFileTree(skill?: string | null) {
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());
  const [childrenByPath, setChildrenByPath] = useState<Record<string, TreeEntry[]>>({});
  const [root, setRoot] = useState<TreeEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loadingPath, setLoadingPath] = useState<string | null>(null);
  /**
   * The folders the cache holds, as a mirror the event handler can read without
   * being a closure over state that has since changed.
   *
   * A ref rather than state because its one reader fires from the backend and
   * needs the *current* truth, not the truth of the render that installed the
   * listener; re-rendering on it would be work nothing depends on.
   */
  const loadedRef = useRef<Set<string>>(new Set());
  /** The root in force, readable from the event handler for the same reason. */
  const skillRef = useRef<string | null | undefined>(skill);
  skillRef.current = skill;

  /**
   * Drops a folder, and everything cached under it, from the tree's own state.
   *
   * A folder whose path stops naming anything -- deleted, renamed, moved --
   * leaves a listing keyed by a path that is now a lie. Keeping it is how a
   * delete ended with the panel reporting a read error for the folder it had just
   * removed: the watcher names the folder's own path from its children's events,
   * and the tree dutifully asked for it again. Forgetting it here means the
   * watcher finds nothing to refresh, and the parent's own re-read removes the
   * row.
   *
   * The subtree goes with it: a folder's descendants are keyed by paths built on
   * the folder's, so none of them can survive it moving.
   */
  const forget = useCallback((path: string) => {
    if (!path) return;
    const prefix = `${path}/`;
    const stale = (key: string) => key === path || key.startsWith(prefix);

    for (const key of Array.from(loadedRef.current)) {
      if (stale(key)) loadedRef.current.delete(key);
    }
    setChildrenByPath((current) => {
      if (!Object.keys(current).some(stale)) return current;
      const next: Record<string, TreeEntry[]> = {};
      for (const [key, entries] of Object.entries(current)) {
        if (!stale(key)) next[key] = entries;
      }
      return next;
    });
    setExpanded((current) => {
      const keys = Array.from(current);
      if (!keys.some(stale)) return current;
      return new Set(keys.filter((key) => !stale(key)));
    });
  }, []);

  /**
   * Whether a folder is no longer in its parent's listing.
   *
   * Asked only after a read has already failed: a folder deleted here, in
   * Explorer or by an agent reports a read error nobody can act on, and the one
   * question worth asking is whether it still exists. Its parent's listing
   * answers that without a second command -- and a parent that is gone too
   * answers it as well, by failing.
   */
  const isGone = useCallback(async (relative: string): Promise<boolean> => {
    try {
      const entries = await listFolder(parentOf(relative), skillRef.current);
      const name = relative.slice(relative.lastIndexOf("/") + 1);
      return !entries.some((entry) => entry.name === name);
    } catch {
      return true;
    }
  }, []);

  const load = useCallback(async (relative: string) => {
    setLoadingPath(relative);
    setError(null);
    try {
      const entries = await listFolder(relative, skillRef.current);
      loadedRef.current.add(relative);
      setChildrenByPath((current) => ({ ...current, [relative]: entries }));
      if (relative === "") {
        setRoot(entries);
      }
      return entries;
    } catch (reason) {
      // A folder that has gone is not an error to show: the tree is about to
      // stop drawing it, and a red line over the whole panel for a folder the
      // user just deleted is the deletion reported back as a failure. The root
      // is never treated this way -- if the workspace itself cannot be read,
      // that is exactly what the user needs to be told.
      if (relative !== "" && (await isGone(relative))) {
        forget(relative);
        return null;
      }
      setError(String(reason));
      loadedRef.current.delete(relative);
      return null;
    } finally {
      setLoadingPath(null);
    }
  }, [forget, isGone]);

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
   * tree is open and the user cannot see that. The mirror forgets with the
   * cache: a folder the tree has dropped must not answer a later event, because
   * there is nothing left to refresh.
   */
  const refresh = useCallback(async () => {
    loadedRef.current = new Set();
    setChildrenByPath({});
    setRoot(null);
    setExpanded(new Set());
    await load("");
  }, [load]);

  /**
   * Reads the root, and opens one folder inside it.
   *
   * Two calls rather than a cascade: the listing says whether the folder is there
   * at all, and its own children are what gets drawn once the row is open. When
   * there is nothing to open this is the plain root read and costs nothing extra.
   *
   * Public because the caller owns *when*: the tree is remounted on a root switch,
   * and the folder to open comes from the click that caused it.
   */
  const openAt = useCallback(async (folder: string | null) => {
    loadedRef.current = new Set();
    setChildrenByPath({});
    setRoot(null);
    setExpanded(new Set());
    const entries = await load("");
    if (!folder || !entries?.some((entry) => entry.isDir && entry.path === folder)) return;
    setExpanded(new Set([folder]));
    await load(folder);
  }, [load]);

  /**
   * Follows the disk while the tree is showing.
   *
   * The backend watches the workspace and emits `panel_fs_changed` with the
   * folders whose listings changed (the root is `""`). Only folders this tree
   * has already drawn are re-fetched: the rest is cache the user is not looking
   * at, and expanding any folder always fetches fresh from disk anyway. The
   * reload is by name, so every *other* folder keeps its open state and its
   * rows exactly as they are.
   *
   * The listener lives with this hook rather than in `FilesTab` because the
   * tree is the state it updates; where this hook is mounted decides whether
   * the panel can hear the disk -- the Files view hides when the panel is
   * switched, and a hidden view has nothing to keep current.
   */
  useEffect(() => {
    let disposed = false;
    void listen<{ folders: string[] }>(WATCH_EVENT, (event) => {
      for (const folder of event.payload?.folders ?? []) {
        // The root is always drawn, so it is always worth a re-read; a folder
        // in the cache is drawn wherever it stands in the tree.
        if (folder === "" || loadedRef.current.has(folder)) void load(folder);
      }
      // The skills root is outside the watched workspace, so the event says
      // nothing about it. Re-read every folder this tree has drawn -- the cached
      // names are exactly those, so a closed folder costs no call -- and only
      // while such a root is showing.
      if (skillRef.current) {
        for (const folder of loadedRef.current) void load(folder);
      }
    }).then((off) => {
      if (disposed) off();
    });
    return () => {
      disposed = true;
    };
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
      await invoke("panel_rename_file", { relative: normalizedPath, newName, skill: skillRef.current ?? null });
      forget(normalizedPath);
      await load(parentOf(normalizedPath));
    } catch (reason) {
      throw new Error(String(reason));
    }
  }, [forget, load]);

  /**
   * Deletes every row a selection names, reporting each one.
   *
   * One command rather than one per row: a selection is several independent
   * items, and asking five times would leave five parent re-reads behind it --
   * while one refusal in the middle must not abandon the rest. The paths are
   * forgotten *before* the listings are re-read, so the watcher's event for a
   * folder that no longer exists finds nothing to ask about (see [`forget`]).
   *
   * @param paths The workspace-relative paths to delete.
   */
  const removeMany = useCallback(async (paths: string[]): Promise<BatchSummary> => {
    const targets = paths.map(normalizePath);
    const summary = await invoke<BatchSummary>("panel_delete_paths", { paths: targets, skill: skillRef.current ?? null });
    for (const path of targets) forget(path);
    for (const folder of new Set(targets.map(parentOf))) await load(folder);
    return summary;
  }, [forget, load]);

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
      await invoke(isDir ? "panel_create_folder" : "panel_create_file", { relative: target, name, skill: skillRef.current ?? null });
      await load(target);
    } catch (reason) {
      throw new Error(String(reason));
    }
  }, [load]);

  /**
   * Moves every row a selection names into one folder, reporting each one.
   *
   * Two listings' worth of work rather than one per item: a move changes two
   * places at once, and every source's own folder has to lose its row. Re-reading
   * the whole tree would do that too, but would also close every folder the user
   * had opened.
   *
   * @param paths The workspace-relative paths being moved.
   * @param targetDir The workspace-relative folder to move them into. `""` is root.
   */
  const moveMany = useCallback(async (paths: string[], targetDir: string): Promise<BatchSummary> => {
    const sources = paths.map(normalizePath);
    const target = normalizePath(targetDir);
    const summary = await invoke<BatchSummary>("panel_move_paths", { paths: sources, target, skill: skillRef.current ?? null });
    for (const source of sources) forget(source);
    const folders = new Set([target, ...sources.map(parentOf)]);
    for (const folder of folders) await load(folder);
    return summary;
  }, [forget, load]);

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
    /**
     * Re-reads one folder's listing, leaving every other folder alone.
     *
     * The tree's own actions call this after they change something; a drop from
     * outside the app does too, because the watcher's own refresh can arrive
     * while the copy is still landing -- and a folder the user just dropped into
     * must show what arrived rather than whatever was there mid-copy.
     */
    reload: load,
    openAt,
    create,
    moveMany,
    rename,
    removeMany,
  };
}
