import { invoke } from "@tauri-apps/api/core";

/**
 * The right panel's durable UI state.
 *
 * `app.right_panel` already holds what the panel *is* (open, width). These keys
 * hold what it was *showing*: which views were open, which folders the Files
 * tree had expanded with whatever file it was previewing, and the page the
 * browser was on. Separate keys rather than one shared object because three
 * places write them at different moments -- the panel's views, the tree, the
 * browser -- and one writer must not overwrite another's field.
 *
 * SQLite, like every other durable preference in the app: a value here is
 * readable from the Database page and cannot disagree with the open/width record
 * beside it.
 */
export const PANEL_VIEWS_KEY = "app.right_panel.views";
export const PANEL_FILES_KEY = "app.right_panel.files";
export const PANEL_BROWSER_KEY = "app.right_panel.browser";

/** What the Files tree remembers for one root (the workspace, or the skills root). */
export type FilesMemoryEntry = {
  /** Workspace-relative folders that were expanded. */
  folders: string[];
  /** The previewed file's relative path, or null when only the tree was showing. */
  file: string | null;
};

/** Every root the tree has been shown on, keyed the way `FilesTab` keys a root. */
export type FilesMemory = Record<string, FilesMemoryEntry>;

/**
 * Reads one panel setting, or null when nothing has been stored yet.
 *
 * Never throws: a missing key, a database that predates the field and a read
 * failure are all "nothing remembered", which must not become an error the user
 * sees for a convenience.
 */
export async function readPanelSetting<T>(key: string): Promise<T | null> {
  try {
    const saved = await invoke<T | null>("database_get_setting", { key });
    return saved ?? null;
  } catch {
    return null;
  }
}

/** Writes one panel setting. A failure is not worth interrupting the user over. */
export function writePanelSetting(key: string, value: unknown): void {
  void invoke("database_set_setting", { key, value }).catch(() => undefined);
}
