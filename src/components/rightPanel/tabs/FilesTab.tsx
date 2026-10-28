import { useCallback, useEffect, useMemo, useRef, useState, type MouseEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { Search } from "lucide-react";
import FileTreeRow from "./FileTreeRow";
import FilePreview from "./FilePreview";
import { CreateNameRow } from "./NameInput";
import ContextMenu from "../menu/ContextMenu";
import { buildFileMenuItems } from "../menu/fileMenu";
import { absolutePath, useFileTree, type TreeEntry } from "../../../features/rightPanel/useFileTree";
import type { FileEdit } from "../../../features/rightPanel/fileEdit";
import { useTreeDrag } from "../../../features/rightPanel/fileTreeDrag";
import { folderAtDropPoint } from "../../../features/rightPanel/externalDrop";
import { workspaceLabel } from "../../../features/workspace/useWorkspaces";

/**
 * The Files view: the workspace as a tree, with a preview of whatever is clicked.
 *
 * Three states in one component rather than three files, because only one is ever
 * visible and they share the same tree state -- a preview that forgot to render
 * its "back" row would strand the user in a file with no way to the tree.
 * Everything expensive is in `useFileTree` and the preview is its own file.
 *
 * The right-click menu and the drag are the two pieces of tree state that are
 * *not* per row: both are owned here and shared by every row, so a create or a
 * drop knows which folder it was aimed at rather than only the row it started on.
 */

type FilesTabProps = {
  /** The open workspace path, or null when none is chosen. */
  workspace: string | null;
  /**
   * An installed skill whose folder this tree draws instead of the workspace.
   *
   * The Skills page sets this to open a skill for editing: a skill is a folder,
   * and the editor the app already has for folders is this tree. When it is set
   * the header names the skill rather than the workspace, and every file command
   * (list, create, rename, move, delete, preview, edit) is scoped to that folder
   * by the backend -- `root_for` in `panel/fs.rs` is the one place that decides.
   *
   * `undefined`/`null` is the workspace. A non-empty string is a skill slug.
   */
  skill?: string | null;
  /** Called by the header's × so the panel can go back to the workspace. */
  onCloseSkill?: () => void;
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

export default function FilesTab({ workspace, skill = null, onCloseSkill, requestedPath, onRequestHandled, onPreviewInBrowser, onNotify }: FilesTabProps) {
  const [preview, setPreview] = useState<TreeEntry | null>(null);
  const [query, setQuery] = useState("");
  const { root, error, loadingPath, isExpanded, childrenOf, toggle, expand, openAt, create, moveMany, removeMany, rename, reload } = useFileTree(skill);

  /**
   * Which tree the previewed file was opened under.
   *
   * A preview is a path relative to a root, so a file shown from the workspace
   * means nothing while the tree is drawing the skills root. Stamping the root
   * lets a switch clear an old preview without clearing one that arrived *with*
   * the switch -- which is exactly what a click on a document in a reply does
   * when the panel was left showing a skill folder.
   */
  const rootKey = `${skill ?? ""}\u0000${workspace ?? ""}`;
  const previewRootRef = useRef<string | null>(null);
  const showPreview = useCallback((entry: TreeEntry) => {
    previewRootRef.current = rootKey;
    setPreview(entry);
  }, [rootKey]);
  const closePreview = useCallback(() => {
    previewRootRef.current = null;
    setPreview(null);
  }, []);

  /** The right-click being answered, or null. `target: null` is the blank space. */
  const [menu, setMenu] = useState<{ point: { x: number; y: number }; target: TreeEntry | null } | null>(null);
  /** The one rename or create field currently open, or null. */
  const [edit, setEdit] = useState<FileEdit | null>(null);

  /**
   * The rows Ctrl/Cmd-clicked, by workspace-relative path.
   *
   * Paths rather than entries: a selection answers "which rows", and every
   * `TreeEntry` is replaced by the next listing. The empty string never appears
   * here -- the root is not a row, and nothing acts on it as one.
   */
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const selectedPaths = useMemo(() => Array.from(selected), [selected]);

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
    showPreview({ name, path: requestedPath, isDir: false, hasChildren: false });
    acknowledgeRef.current();
    // `showPreview` is re-created only when the root changes. The request is
    // consumed before any later root switch, so this cannot loop.
  }, [requestedPath, showPreview]);

  // Re-read from the root whenever the folder changes. Without this the tree
  // would keep showing the *previous* workspace's contents next to the new
  // folder's name in the header, which is worse than an empty panel.
  //
  // **Reset only on a genuine switch.** This effect and the request effect above
  // both fire on mount when the panel opens *because* of a request -- that is the
  // normal path for clicking a changed file. Running the reset unconditionally
  // set the preview and cleared it in the same commit, so the first click appeared
  // to do nothing and only the second worked. The sentinel is what tells the two
  // apart: it starts as a value the root can never be, so the first render always
  // counts as "just mounted" rather than as "the folder is still null".
  //
  // **The root is the workspace *or* the skills root.** Opening a skill is a
  // switch to a different tree, so it takes the same path as changing folder:
  // everything open against the old one goes. The key is the pair, because the
  // skills root is drawn regardless of which workspace happens to be open.
  //
  // **A skill opens its folder, not the tree around it.** When `skill` names one,
  // its row is expanded on entry so the files are the first thing on screen --
  // while its siblings stay listed, because the skills root is a folder like any
  // other and a person may well want the neighbouring one.
  const loadedRootRef = useRef<string | null | undefined>(undefined);

  useEffect(() => {
    const previous = loadedRootRef.current;
    loadedRootRef.current = rootKey;
    if (previous !== undefined) {
      // A genuine switch: anything open against the old folder is stale, so the
      // preview and the filter go and the tree is read again for the new one.
      if (previous === rootKey) return;
      // ...unless the preview was opened under the root being switched *to*.
      // That is a file requested from a reply while a skill was showing: the
      // request and the root change arrive in one commit, and clearing here
      // would throw away the file the click asked for.
      if (previewRootRef.current !== null && previewRootRef.current !== rootKey) {
        closePreview();
      }
      setQuery("");
      setEdit(null);
      setMenu(null);
      // A selection names paths in the tree that is being left, so none of it
      // can mean anything in the one arriving.
      setSelected(new Set());
    }
    // First run: read the tree, but leave a requested file alone. It is on its way
    // in from the click that opened this panel, and clearing it here is what made
    // the first click on a changed file look lost.
    void openAt(skill ?? null);
  }, [rootKey, skill, openAt, closePreview]);

  const closeMenu = useCallback(() => setMenu(null), []);

  // A row's right-click is answered here, and stopped from bubbling so the blank
  // space underneath does not claim it as its own.
  const openRowMenu = useCallback((entry: TreeEntry, event: MouseEvent) => {
    event.preventDefault();
    event.stopPropagation();
    // The menu acts on what is highlighted. A right-click on a row outside the
    // selection makes that row the selection, so Delete can never act on rows the
    // pointer is not on.
    setSelected((current) => (current.has(entry.path) ? current : new Set([entry.path])));
    setMenu({ point: { x: event.clientX, y: event.clientY }, target: entry });
  }, []);

  const openBlankMenu = useCallback((event: MouseEvent) => {
    event.preventDefault();
    setMenu({ point: { x: event.clientX, y: event.clientY }, target: null });
  }, []);

  const startRename = useCallback((entry: TreeEntry) => {
    setEdit({ kind: "rename", path: entry.path, name: entry.name, isDir: entry.isDir });
  }, []);

  const startCreate = useCallback(async (parent: TreeEntry | null, isDir: boolean) => {
    const parentPath = parent?.path ?? "";
    // A folder has to be open for the new row to land visibly inside it. The
    // root is always drawn, so only a real folder needs expanding.
    if (parentPath) await expand(parentPath);
    setEdit({ kind: "create", parent: parentPath, isDir });
  }, [expand]);

  /**
   * Deletes the given paths, after one confirmation for the whole selection.
   *
   * `described` is the sentence's object -- one name, or a count -- because what
   * is about to go is the only thing to judge before an unrecoverable press.
   * Refusals are reported per item, since the backend answers per item: one
   * protected path must not hide the four that were deleted.
   */
  const runDelete = useCallback(async (paths: string[], described: string) => {
    if (paths.length === 0) return;
    if (!window.confirm(`Delete ${described}?`)) return;
    try {
      const summary = await removeMany(paths);
      for (const refusal of summary.refused) onNotify?.("error", refusal);
      setSelected(new Set());
    } catch (reason) {
      onNotify?.("error", `Could not delete: ${String(reason)}`);
    }
  }, [removeMany, onNotify]);

  /**
   * The menu's destructive row.
   *
   * It deletes the selection when the row it was opened on is part of one and
   * there is more than that row -- otherwise just the row, so acting on a single
   * file never depends on what happened to be selected before.
   */
  const deleteFromMenu = useCallback((entry: TreeEntry) => {
    if (selectedPaths.length > 1 && selectedPaths.includes(entry.path)) {
      void runDelete(selectedPaths, `${selectedPaths.length} selected items, folders with everything inside them`);
      return;
    }
    void runDelete([entry.path], entry.isDir ? `folder "${entry.name}" and everything in it` : `"${entry.name}"`);
  }, [selectedPaths, runDelete]);

  /**
   * Delete on the selection.
   *
   * Bound to the document because a row is a button and the tree has no single
   * focusable list: the key would otherwise work only while a row happened to
   * have focus, which is not how anyone selects five files and presses Delete.
   * Skipped while a field holds the caret (rename, filter, a file's source) and
   * while a dialog is open, or Delete inside Settings would reach the files
   * behind it. Nothing is selected while a preview is showing, so that view is
   * left alone too.
   */
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Delete" || preview) return;
      const target = event.target as HTMLElement | null;
      if (target?.closest("input, textarea, [contenteditable='true']")) return;
      if (document.querySelector("dialog[open]")) return;
      if (selectedPaths.length === 0) return;
      event.preventDefault();
      const described = selectedPaths.length === 1
        ? `"${selectedPaths[0].slice(selectedPaths[0].lastIndexOf("/") + 1)}"`
        : `${selectedPaths.length} selected items, folders with everything inside them`;
      void runDelete(selectedPaths, described);
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [preview, selectedPaths, runDelete]);

  /**
   * Puts the entry's absolute path on the clipboard.
   *
   * Absolute rather than the tree-relative path, because that is what "the path"
   * means to everything outside this app -- a terminal, an editor, another
   * window -- and the tree-relative form is only meaningful back inside it.
   */
  const copyPath = useCallback(async (entry: TreeEntry) => {
    if (!workspace) return;
    try {
      await navigator.clipboard.writeText(absolutePath(workspace, entry.path));
      onNotify?.("success", "Path copied");
    } catch {
      onNotify?.("error", "Could not copy the path");
    }
  }, [workspace, onNotify]);

  /**
   * Applies the open field.
   *
   * On failure the field is left open: the name is the thing that was wrong, and
   * making the user reopen the menu to retype it would be the wrong lesson.
   */
  const commitEdit = useCallback(async (name: string) => {
    if (!edit) return;
    try {
      if (edit.kind === "rename") {
        if (name === edit.name) { setEdit(null); return; }
        await rename(edit.path, name);
      } else {
        await create(edit.parent, name, edit.isDir);
      }
      setEdit(null);
    } catch (reason) {
      onNotify?.("error", (reason as Error).message);
    }
  }, [edit, rename, create, onNotify]);

  const cancelEdit = useCallback(() => setEdit(null), []);

  /**
   * Moves what a drag carried into the folder it was dropped on.
   *
   * One command for the whole selection, per-item answers for the ones that could
   * not come: a name already taken must not undo the rest of the move. Failures
   * reach the notification stack rather than being swallowed, because a row that
   * just stays put reads as a bug.
   */
  const dropTo = useCallback(async (sources: string[], targetDir: string) => {
    try {
      const summary = await moveMany(sources, targetDir);
      for (const refusal of summary.refused) onNotify?.("error", refusal);
      if (summary.done.length > 0) setSelected(new Set());
    } catch (reason) {
      onNotify?.("error", String(reason));
    }
  }, [moveMany, onNotify]);

  // One drag state for the whole tree, handed to every row: which rows are
  // carried, which folder they are over, and the press that begins it. The
  // validity rules and the pointer tracking live in the hook, so this file stays
  // about the tree.
  const treeDrag = useTreeDrag((sources, targetDir) => void dropTo(sources, targetDir));

  /** The folder an OS drag is over, so a file coming in lights up the same row a move does. */
  const [osDropFolder, setOsDropFolder] = useState<string | undefined>(undefined);

  /**
   * Copies what was dropped in from Explorer into the folder under the pointer.
   *
   * Per item, because the backend answers per item: one name already taken must
   * not throw away the rest of the drop. The folder is re-read after the copy
   * rather than left to the watcher -- the watcher can report a folder while the
   * files are still being written, and a drop that shows nothing has arrived
   * reads as a drop that failed.
   */
  const importDropped = useCallback(async (paths: string[], folder: string) => {
    if (paths.length === 0) return;
    try {
      const summary = await invoke<{ imported: string[]; refused: string[] }>("panel_import_paths", {
        paths,
        target: folder,
        skill,
      });
      for (const refusal of summary.refused) onNotify?.("error", refusal);
      if (summary.imported.length === 0) return;
      onNotify?.(
        "success",
        summary.imported.length === 1
          ? `Copied ${summary.imported[0]}`
          : `Copied ${summary.imported.length} items`,
      );
      // The target may be a row that was closed, so opening it is part of
      // showing what the drop did.
      await expand(folder);
      await reload(folder);
    } catch (reason) {
      onNotify?.("error", `Could not copy here: ${String(reason)}`);
    }
  }, [skill, onNotify, expand, reload]);

  /**
   * Files dropped from outside the app, while this view is the one showing.
   *
   * Tauri consumes the OS drag to implement its own file drop -- which is also
   * why the tree's own move is a pointer drag rather than HTML5 -- so the paths
   * arrive through `onDragDropEvent` and not a DOM `drop`. The position is in
   * physical pixels, so it is divided by `devicePixelRatio` before it is used to
   * find the row under the pointer.
   *
   * The listener lives here rather than in the panel shell because the tree is
   * what a drop is aimed at: with Files closed there is no tree to drop on, and
   * the panel's other views must not swallow a file drag.
   */
  useEffect(() => {
    let live = true;
    let stop: (() => void) | undefined;
    const point = (position: { x: number; y: number }) => {
      const scale = window.devicePixelRatio || 1;
      return { x: position.x / scale, y: position.y / scale };
    };

    void getCurrentWebview()
      .onDragDropEvent((event) => {
        const payload = event.payload;
        if (payload.type === "leave") {
          setOsDropFolder(undefined);
          return;
        }
        if (payload.type === "drop") {
          const at = point(payload.position);
          const folder = folderAtDropPoint(at.x, at.y);
          setOsDropFolder(undefined);
          if (folder !== undefined) void importDropped(payload.paths, folder);
          return;
        }
        if (payload.type === "enter" || payload.type === "over") {
          const at = point(payload.position);
          setOsDropFolder(folderAtDropPoint(at.x, at.y));
        }
      })
      .then((unlisten) => {
        if (live) stop = unlisten;
        else unlisten();
      });

    return () => {
      live = false;
      stop?.();
    };
  }, [importDropped]);

  /**
   * A click on a row's name.
   *
   * A click that is the tail of a drag is ignored -- the drag already did what
   * the press was for.
   *
   * Plain click is the tree's existing gesture -- open a file, expand a folder --
   * and drops any selection, because acting on rows the user has just clicked
   * away from is acting on something they cannot see is still selected.
   * Ctrl/Cmd-click toggles one row's membership instead and opens nothing: the
   * point of it is to gather several rows before a drag or a Delete.
   */
  const activateEntry = useCallback((entry: TreeEntry, event: MouseEvent) => {
    if (treeDrag.shouldIgnoreClick()) return;
    if (event.ctrlKey || event.metaKey) {
      setSelected((current) => {
        const next = new Set(current);
        if (next.has(entry.path)) next.delete(entry.path);
        else next.add(entry.path);
        return next;
      });
      return;
    }
    setSelected(new Set());
    if (entry.isDir) void toggle(entry.path);
    else showPreview(entry);
  }, [treeDrag, toggle, showPreview]);

  const toggleEntry = useCallback((path: string) => {
    if (treeDrag.shouldIgnoreClick()) return;
    void toggle(path);
  }, [treeDrag, toggle]);

  /**
   * Shows the entry's folder in the OS file manager, with the row selected there.
   *
   * Done by the backend rather than the opener plugin's JavaScript half, so it
   * works for either root -- a skill's folder as well as the workspace -- without
   * this file ever learning where the skills root is.
   */
  const reveal = useCallback(async (entry: TreeEntry) => {
    try {
      await invoke("panel_reveal_path", { relative: entry.path, skill });
    } catch (reason) {
      onNotify?.("error", String(reason));
    }
  }, [skill, onNotify]);

  const items = useMemo(() => {
    const built = buildFileMenuItems(menu?.target ?? null, {
      onView: showPreview,
      onCopyPath: (entry) => void copyPath(entry),
      onReveal: (entry) => void reveal(entry),
      onRename: startRename,
      onDelete: deleteFromMenu,
      onCreateFile: (parent) => void startCreate(parent, false),
      onCreateFolder: (parent) => void startCreate(parent, true),
    });
    // A selection of several rows says so on the one destructive row: "Delete"
    // beside a menu opened on one of five would read as "delete this one".
    const target = menu?.target;
    if (target && selectedPaths.length > 1 && selectedPaths.includes(target.path)) {
      return built.map((item) =>
        item.id === "delete" ? { ...item, label: `Delete ${selectedPaths.length} items` } : item,
      );
    }
    return built;
  }, [menu, copyPath, reveal, startRename, deleteFromMenu, startCreate, showPreview, selectedPaths]);

  // A file opened under a folder that has since changed may no longer exist, so
  // returning to the tree is the only way to see what is there now.
  //
  // The preview is handed the skill as well as the workspace: a `.png` in a
  // skill's `assets/` is read through the same bytes command, and that command
  // needs to know which root the path is relative to.
  if (preview) {
    return <FilePreview entry={preview} workspace={workspace} skill={skill} onBack={closePreview} onPreviewInBrowser={onPreviewInBrowser} onNotify={onNotify} />;
  }

  if (!workspace && !skill) {
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

  // Two drags, one highlight: a row being carried to another folder, and a file
  // arriving from outside the app. `??` rather than `||`, because the empty
  // string is the root and is a real target.
  const dropTarget = osDropFolder ?? treeDrag.target;

  return (
    // `data-os-drop-root` marks the whole view as somewhere a file from Explorer
    // can land (the root folder). It is deliberately not `data-drop-root`: that
    // one is also the tree's own move target, and a move onto the header should
    // still mean "nowhere" rather than silently mean the root.
    <div data-os-drop-root className="flex min-h-0 flex-1 flex-col">
      {/* Which folder is being shown, then search. The skills root says so and
          offers the way back to the workspace, because it replaced that tree
          rather than opening beneath it -- and without a door back, the panel
          would look like it had lost the project. */}
      <div className="flex min-h-10 shrink-0 items-center gap-2 border-b border-[var(--line)] px-3">
        {skill ? (
          <>
            <span className="min-w-0 flex-1 truncate text-[13px] font-medium text-[var(--accent)]" title="skills">
              skills
            </span>
            <button
              type="button"
              onClick={onCloseSkill}
              className="shrink-0 rounded px-1.5 py-0.5 text-[11px] text-[var(--muted)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
            >
              Close
            </button>
          </>
        ) : (
          <span className="min-w-0 flex-1 truncate text-[13px] font-medium text-[var(--accent)]" title={workspace ?? undefined}>{workspace ? workspaceLabel(workspace) : ""}</span>
        )}
        <Search size={13} className="shrink-0 text-[var(--quiet)]" />
        <input
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder="Filter"
          aria-label="Filter files"
          className="w-16 shrink-0 rounded bg-transparent text-right text-xs text-[var(--text)] outline-none placeholder:text-[var(--quiet)] focus:w-28 focus:text-left"
        />
      </div>

      {/* The whole column answers a right-click on its blank space with the root's
          creates, and is the drop target that means "move to the root". The mark
          is what both drags read under the pointer -- the tree's own move, which
          tracks the pointer globally in `useTreeDrag`, and a file dropped in from
          Explorer, whose position comes from Tauri and is resolved by
          `folderAtDropPoint`. */}
      <div
        data-drop-root
        className={`min-h-0 flex-1 overflow-y-auto py-1 ${dropTarget === "" ? "bg-[color-mix(in_srgb,var(--accent)_6%,transparent)]" : ""}`}
        onContextMenu={openBlankMenu}
        // Only the blank space itself: a click on a row is that row's, and this
        // fires after it, so a `contains`-style check would clear the selection
        // the click had just made.
        onClick={(event) => { if (event.target === event.currentTarget) setSelected(new Set()); }}
      >
        {error && <p className="px-3 py-2 text-xs text-[var(--danger)]">{error}</p>}
        {filtered === null && !error && <p className="px-3 py-2 text-xs text-[var(--quiet)]">Reading…</p>}
        {filtered?.length === 0 && <p className="px-3 py-2 text-xs text-[var(--quiet)]">Nothing here.</p>}
        {/* A create aimed at the root has no row to hang off, so it is drawn as
            the first thing in the list. */}
        {edit?.kind === "create" && edit.parent === "" && (
          <CreateNameRow depth={0} isDir={edit.isDir} onCommit={commitEdit} onCancel={cancelEdit} />
        )}
        {filtered?.map((entry) => (
            <FileTreeRow
              key={entry.path}
              entry={entry}
              depth={0}
              isExpanded={isExpanded}
              childrenOf={childrenOf}
              loadingPath={loadingPath}
              onToggle={toggleEntry}
              onActivate={activateEntry}
              onContextMenu={openRowMenu}
              selection={selected}
              edit={edit}
              onCommitEdit={commitEdit}
              onCancelEdit={cancelEdit}
              drag={{ ...treeDrag, target: dropTarget }}
            />
          ))}
      </div>

      {menu && (
        <ContextMenu
          point={menu.point}
          label={menu.target ? `${menu.target.name} actions` : "File actions"}
          items={items}
          onClose={closeMenu}
        />
      )}
    </div>
  );
}
