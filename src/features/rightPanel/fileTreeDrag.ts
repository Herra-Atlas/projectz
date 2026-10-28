import { useCallback, useEffect, useRef, useState } from "react";
import type { PointerEvent as ReactPointerEvent } from "react";
import { parentOf, type TreeEntry } from "./useFileTree";

/** How far the pointer must travel before a press counts as a drag, not a click. */
const THRESHOLD = 4;

/**
 * Drag state a row needs, and the one handler that starts a drag from it.
 *
 * `target` is the folder under the pointer -- a path, `""` for the root area, or
 * `undefined` when nothing valid is hovered -- and `sources` are the rows being
 * carried (one, or a whole selection), empty when nothing is.
 */
export type TreeDrag = {
  target: string | undefined;
  sources: string[];
  /**
   * Begin watching a press on a row; it becomes a drag once it moves far enough.
   *
   * `carried` is the selection to carry when the pressed row is part of one. The
   * row does not know the selection, so the caller that does hands it over at
   * press time; pressing a row outside the selection carries just that row.
   */
  onPointerDown: (entry: TreeEntry, event: ReactPointerEvent, carried?: string[]) => void;
};

/**
 * Whether one entry may be dropped into `targetDir`.
 *
 * Refused when it would be a no-op (the folder it already lives in), a move onto
 * itself, or a move into its own subtree. The backend refuses these too, but a
 * target that would be rejected must not light up as though it would work.
 */
function canMove(source: string, targetDir: string): boolean {
  if (!source) return false;
  if (targetDir === source) return false;
  if (targetDir.startsWith(`${source}/`)) return false;
  if (parentOf(source) === targetDir) return false;
  return true;
}

/**
 * Whether a selection may be dropped into `targetDir`.
 *
 * **Some, not all.** A folder lights up when anything under the pointer would
 * actually move: dragging three files onto a folder one of them already lives in
 * is a move of the other two, which is what a file manager does with the same
 * gesture. The rows that cannot move are dropped from the carried set at the
 * moment of the drop rather than turning the whole drag into a refusal.
 */
export function canDrop(sources: string[], targetDir: string): boolean {
  return sources.some((source) => canMove(source, targetDir));
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
export function useTreeDrag(onDrop: (sources: string[], targetDir: string) => void): TreeDrag & { shouldIgnoreClick: () => boolean } {
  const [sources, setSources] = useState<string[]>([]);
  const [target, setTarget] = useState<string | undefined>(undefined);

  const pending = useRef<{ paths: string[]; x: number; y: number } | null>(null);
  const active = useRef(false);
  // Mirrors read from the document listeners. Those are subscribed once, so they
  // must never close over a stale target or a stale callback.
  const targetRef = useRef<string | undefined>(undefined);
  const sourcesRef = useRef<string[]>([]);
  const dropRef = useRef(onDrop);
  // Set when a drag ends, so the click that follows is not read as "open this".
  const ignoreClick = useRef(false);

  dropRef.current = onDrop;

  const setCarried = useCallback((next: string[]) => {
    sourcesRef.current = next;
    setSources(next);
  }, []);

  const setDropTarget = useCallback((next: string | undefined) => {
    if (targetRef.current === next) return;
    targetRef.current = next;
    setTarget(next);
  }, []);

  /** The drop target under a viewport point, or undefined when there is none. */
  const targetAt = useCallback((x: number, y: number, carried: string[]): string | undefined => {
    const element = document.elementFromPoint(x, y) as HTMLElement | null;
    if (!element) return undefined;
    const row = element.closest<HTMLElement>("[data-entry-path]");
    if (row) {
      // A row always wins over the root area behind it, even a file that cannot
      // receive the drop -- otherwise hovering a file would quietly mean "move to
      // the root", which is not what the pointer is on.
      if (row.dataset.entryDir !== "true") return undefined;
      const path = row.dataset.entryPath ?? "";
      return canDrop(carried, path) ? path : undefined;
    }
    if (element.closest("[data-drop-root]")) return canDrop(carried, "") ? "" : undefined;
    return undefined;
  }, []);

  const onPointerDown = useCallback((entry: TreeEntry, event: ReactPointerEvent, carried?: string[]) => {
    if (event.button !== 0) return;
    // A press inside a text field is for editing, not carrying. The rename and
    // create fields live inside rows, so this is exactly where a stray drag would
    // otherwise start.
    if ((event.target as HTMLElement).closest("input, textarea")) return;
    ignoreClick.current = false;
    pending.current = {
      paths: carried && carried.length > 0 ? carried : [entry.path],
      x: event.clientX,
      y: event.clientY,
    };
    active.current = false;
  }, []);

  useEffect(() => {
    const move = (event: PointerEvent) => {
      const start = pending.current;
      if (!start) return;
      if (!active.current) {
        if (Math.hypot(event.clientX - start.x, event.clientY - start.y) < THRESHOLD) return;
        active.current = true;
        setCarried(start.paths);
        // No text selection or I-beam while a row is being carried around.
        document.body.style.userSelect = "none";
        document.body.style.cursor = "grabbing";
      }
      setDropTarget(targetAt(event.clientX, event.clientY, start.paths));
    };

    const finish = (event: PointerEvent) => {
      const start = pending.current;
      if (!start) return;
      const wasActive = active.current;
      pending.current = null;
      active.current = false;
      document.body.style.userSelect = "";
      document.body.style.cursor = "";
      const dropped = wasActive ? targetAt(event.clientX, event.clientY, start.paths) : undefined;
      setCarried([]);
      setDropTarget(undefined);
      if (wasActive && dropped !== undefined) {
        // A drag finishes with a click on whatever the press began over; that
        // click is not a request to open a file.
        ignoreClick.current = true;
        const carried = start.paths.filter((source) => canMove(source, dropped));
        if (carried.length > 0) dropRef.current(carried, dropped);
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
  }, [setCarried, setDropTarget, targetAt]);

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

  return { target, sources, onPointerDown, shouldIgnoreClick };
}
