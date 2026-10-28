import { parentOf } from "./useFileTree";

/**
 * The folder a file dropped at this viewport point belongs in.
 *
 * A drop from Explorer arrives as a *position* -- Tauri hands over the pointer in
 * physical pixels and nothing else about where it was aimed -- so the target is
 * read back off the same `data-*` marks the tree's own drag uses: a folder row is
 * itself, a file row means the folder it sits in, and anywhere else in the tree's
 * body is the root. `undefined` means the pointer is not over the tree at all.
 *
 * Coordinates are CSS pixels. Both callers divide the event's position by
 * `devicePixelRatio` first, because a physical-pixel point on a scaled display
 * lands on the wrong row.
 */
export function folderAtDropPoint(x: number, y: number): string | undefined {
  const element = document.elementFromPoint(x, y) as HTMLElement | null;
  if (!element) return undefined;

  const row = element.closest<HTMLElement>("[data-entry-path]");
  if (row) {
    const path = row.dataset.entryPath ?? "";
    // A file cannot receive anything, but dropping on one and meaning its folder
    // is the obvious reading -- unlike the tree's own move, where a file row
    // deliberately means "not a target" so a near-miss cannot become a move to
    // the root.
    return row.dataset.entryDir === "true" ? path : parentOf(path);
  }
  return element.closest("[data-os-drop-root], [data-drop-root]") ? "" : undefined;
}

/**
 * Whether an OS drop at this point landed on the right panel.
 *
 * The panel's own listener is the one that acts on such a drop, but the chat
 * page's listener is window-wide: without this, a file dropped on the Files tree
 * would be copied into the workspace *and* attached to the composer, which is
 * one gesture answered twice. Only the panel's frame is claimed -- a drop on the
 * chat itself still attaches, as it always did.
 */
export function isOverRightPanel(x: number, y: number): boolean {
  const element = document.elementFromPoint(x, y) as HTMLElement | null;
  return Boolean(element?.closest("[data-right-panel]"));
}
