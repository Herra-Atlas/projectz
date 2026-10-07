import { useCallback, useEffect, useRef, useState } from "react";
import type { PointerEvent as ReactPointerEvent } from "react";
import { parentOf, type TreeEntry } from "./useFileTree";

/** How far the pointer must travel before a press counts as a drag, not a click. */
const THRESHOLD = 4;

/**
 * Drag state a row needs, and the one handler that starts a drag from it.
 *
 * `target` is the folder under the pointer -- a path, `""` for the root area, or
 * `undefined` when nothing valid is hovered -- and `source` is the row being
 * carried, or null.
 */
export type TreeDrag = {
  target: string | undefined;
  source: string | null;
  /** Begin watching a press on a row; it becomes a drag once it moves far enough. */
  onPointerDown: (entry: TreeEntry, event: ReactPointerEvent) => void;
};

/**
 * Whether the entry at `source` may be dropped into `targetDir`.
 *
 * Refused when it would be a no-op (the folder it already lives in), a move onto
 * itself, or a move into its own subtree. The backend refuses these too, but a
 * target that would be rejected must not light up as though it would work.
 */
export function canDrop(source: string, targetDir: string): boolean {
  if (!source) return false;
  if (targetDir === source) return false;
  if (targetDir.startsWith(`${source}/`)) return false;
  if (parentOf(source) === targetDir) return false;
  return true;
}

/**
 * Drag-to-move for the file tree, driven by pointer events rather than HTML5
 * drag-and-drop.
 *
 * **Not HTML5 DnD, on purpose.** This app keeps Tauri's native file-drop on so a
 * file dragged in from the OS can be attached in the chat, and on Windows that
 * same native handler is what keeps the webview's own `dragstart`/`drop` from
 * firing. Pointer events are oblivious to it, so the row drag and the OS file
 * drop coexist instead of one silencing the other.
 *
 * A press only becomes a drag after the pointer moves past a small threshold, so
 * an ordinary click -- which opens a file -- is never mistaken for a drag. The
 * target under the pointer is read from the `data-*` marks the rows carry, and
 * state is only set when the target actually changes, so carrying a row does not
 * re-render the tree on every pointer move.
 */
export function useTreeDrag(onDrop: (source: string, targetDir: string) => void): TreeDrag & { shouldIgnoreClick: () => boolean } {
  const [source, setSource] = useState<string | null>(null);
  const [target, setTarget] = useState<string | undefined>(undefined);

  const pending = useRef<{ path: string; x: number; y: number } | null>(null);
  const active = useRef(false);
  // Mirrors read from the document listeners. Those are subscribed once, so they
  // must never close over a stale target or a stale callback.
  const targetRef = useRef<string | undefined>(undefined);
  const dropRef = useRef(onDrop);
  // Set when a drag ends, so the click that follows is not read as "open this".
  const ignoreClick = useRef(false);

  dropRef.current = onDrop;

  const setDropTarget = useCallback((next: string | undefined) => {
    if (targetRef.current === next) return;
    targetRef.current = next;
    setTarget(next);
  }, []);

  /** The drop target under a viewport point, or undefined when there is none. */
  const targetAt = useCallback((x: number, y: number, sourcePath: string): string | undefined => {
    const element = document.elementFromPoint(x, y) as HTMLElement | null;
    if (!element) return undefined;
    const row = element.closest<HTMLElement>("[data-entry-path]");
    if (row) {
      // A row always wins over the root area behind it, even a file that cannot
      // receive the drop -- otherwise hovering a file would quietly mean "move to
      // the root", which is not what the pointer is on.
      if (row.dataset.entryDir !== "true") return undefined;
      const path = row.dataset.entryPath ?? "";
      return canDrop(sourcePath, path) ? path : undefined;
    }
    if (element.closest("[data-drop-root]")) return canDrop(sourcePath, "") ? "" : undefined;
    return undefined;
  }, []);

  const onPointerDown = useCallback((entry: TreeEntry, event: ReactPointerEvent) => {
    if (event.button !== 0) return;
    // A press inside a text field is for editing, not carrying. The rename and
    // create fields live inside rows, so this is exactly where a stray drag would
    // otherwise start.
    if ((event.target as HTMLElement).closest("input, textarea")) return;
    ignoreClick.current = false;
    pending.current = { path: entry.path, x: event.clientX, y: event.clientY };
    active.current = false;
  }, []);

  useEffect(() => {
    const move = (event: PointerEvent) => {
      const start = pending.current;
      if (!start) return;
      if (!active.current) {
        if (Math.hypot(event.clientX - start.x, event.clientY - start.y) < THRESHOLD) return;
        active.current = true;
        setSource(start.path);
        // No text selection or I-beam while a row is being carried around.
        document.body.style.userSelect = "none";
        document.body.style.cursor = "grabbing";
      }
      setDropTarget(targetAt(event.clientX, event.clientY, start.path));
    };

    const finish = (event: PointerEvent) => {
      const start = pending.current;
      if (!start) return;
      const wasActive = active.current;
      pending.current = null;
      active.current = false;
      document.body.style.userSelect = "";
      document.body.style.cursor = "";
      const dropped = wasActive ? targetAt(event.clientX, event.clientY, start.path) : undefined;
      setSource(null);
      setDropTarget(undefined);
      if (wasActive && dropped !== undefined) {
        // A drag finishes with a click on whatever the press began over; that
        // click is not a request to open a file.
        ignoreClick.current = true;
        dropRef.current(start.path, dropped);
      }
    };

    document.addEventListener("pointermove", move);
    document.addEventListener("pointerup", finish);
    document.addEventListener("pointercancel", finish);
    return () => {
      document.body.style.userSelect = "";
      document.body.style.cursor = "";
      document.removeEventListener("pointermove", move);
      document.removeEventListener("pointerup", finish);
      document.removeEventListener("pointercancel", finish);
    };
  }, [setDropTarget, targetAt]);

  /**
   * Reads and clears the "the last gesture was a drag" flag. A click handler
   * calls this first; true means the click was the tail of a drag and should be
   * ignored.
   */
  const shouldIgnoreClick = useCallback(() => {
    const ignore = ignoreClick.current;
    ignoreClick.current = false;
    return ignore;
  }, []);

  return { target, source, onPointerDown, shouldIgnoreClick };
}
