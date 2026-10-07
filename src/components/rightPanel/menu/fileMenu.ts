import { Eye, FilePlus, FolderPlus, Pencil, Trash2 } from "lucide-react";
import type { TreeEntry } from "../../../features/rightPanel/useFileTree";
import type { ContextMenuItem } from "./ContextMenu";

type FileMenuHandlers = {
  /** A file's path, opened in the preview. */
  onView: (entry: TreeEntry) => void;
  onRename: (entry: TreeEntry) => void;
  onDelete: (entry: TreeEntry) => void;
  /** The folder to create inside, or null for the workspace root. */
  onCreateFile: (parent: TreeEntry | null) => void;
  onCreateFolder: (parent: TreeEntry | null) => void;
};

/**
 * The actions offered by a right-click in the Files view.
 *
 * One builder for all three targets -- a file, a folder, and the blank space
 * under the tree -- because it is one menu and only the applicable rows differ:
 *
 * - a file   -> View, Rename, Delete
 * - a folder -> Create file, Create folder, Rename, Delete
 * - the space -> Create file, Create folder
 *
 * `target` is null for the blank space, which *means the workspace root*: there
 * is no row to name but there is still a folder to create into. Reading null as
 * "the root" rather than "nothing" is what lets the two surfaces share a builder.
 */
export function buildFileMenuItems(target: TreeEntry | null, handlers: FileMenuHandlers): ContextMenuItem[] {
  const items: ContextMenuItem[] = [];

  // A file's one action is to look at it; a folder holds no text to view.
  if (target && !target.isDir) {
    items.push({ id: "view", label: "View", icon: Eye, onSelect: () => handlers.onView(target) });
  }

  // Folders and the root both create *inside* something, so they share these
  // two rows. A file can contain nothing.
  if (!target || target.isDir) {
    items.push({ id: "create-file", label: "Create file", icon: FilePlus, onSelect: () => handlers.onCreateFile(target) });
    items.push({ id: "create-folder", label: "Create folder", icon: FolderPlus, onSelect: () => handlers.onCreateFolder(target) });
  }

  // Only a real row can be renamed or deleted; the blank space is not a thing.
  if (target) {
    items.push({ id: "rename", label: "Rename", icon: Pencil, separated: items.length > 0, onSelect: () => handlers.onRename(target) });
    items.push({ id: "delete", label: "Delete", icon: Trash2, tone: "danger", onSelect: () => handlers.onDelete(target) });
  }

  return items;
}
