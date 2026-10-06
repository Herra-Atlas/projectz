import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

/**
 * The open workspaces and which one the tools are working in.
 *
 * Mirrors the wire shape of `database::workspaces::Workspaces` exactly: the list
 * is ordered most-recent-first and the selection is a path that is either in the
 * list or absent. Keeping the two the same shape is what lets the header and the
 * picker agree without either of them re-deriving the other's fields.
 */
export type WorkspaceState = {
  paths: string[];
  selected: string | null;
};

/**
 * The folder name shown in place of a path.
 *
 * The same rule VS Code uses, for the same reason: a list of full paths cannot be
 * scanned, and every entry would be the same width. A drive root has no final
 * segment, so the whole path stands in rather than rendering an empty row.
 */
export const workspaceLabel = (path: string) => {
  const trimmed = path.replace(/[\\/]+$/, "");
  const name = trimmed.split(/[\\/]/).pop();
  return name && name.length > 0 ? name : path;
};

const EMPTY: WorkspaceState = { paths: [], selected: null };

/**
 * Loads the open workspaces once the app has started and exposes the three
 * actions the picker offers.
 *
 * Every mutation goes through a command rather than writing the setting
 * directly, because the backend has to move the tools' root in the *same* step as
 * it records the choice. A frontend that saved the setting itself would produce a
 * header showing one folder while every tool path resolved against another, and
 * nothing about that failure would be visible.
 */
export function useWorkspaces(enabled: boolean) {
  const [state, setState] = useState<WorkspaceState>(EMPTY);
  const [error, setError] = useState("");

  /**
   * Reads the saved list, and nothing else.
   *
   * Only the list, because that is the only thing the UI renders. The live root
   * is deliberately not read here: with no default it is empty exactly when
   * nothing is chosen, so a header that fell back to it would describe a second
   * state the user never picked. The backend moves the root in the same step it
   * records the choice, which is what keeps the two from disagreeing.
   */
  const sync = useCallback(async () => {
    const saved = await invoke<WorkspaceState>("workspaces_list");
    setState({ paths: saved?.paths ?? [], selected: saved?.selected ?? null });
    setError("");
  }, []);

  useEffect(() => {
    if (!enabled) return;
    let mounted = true;
    // `sync` sets state, so it cannot be called directly here without a guard on
    // an unmounted component; the flag is what makes the promise safe to ignore.
    void (async () => {
      try {
        await sync();
      } catch (reason: unknown) {
        if (mounted) setError(String(reason));
      }
    })();
    return () => { mounted = false; };
  }, [enabled, sync]);

  /**
   * Runs a command, then re-reads the list rather than trusting its return.
   *
   * **Never rethrows.** A failure is recorded in `error` and rendered by the
   * header instead, so a caller can close its menu unconditionally. Rethrowing
   * would leave every caller needing a `.catch` purely to avoid an unhandled
   * rejection, and the one thing it would buy -- knowing the call failed -- is
   * already in the error the header shows.
   */
  const run = useCallback(async (command: string, path: string) => {
    try {
      await invoke(command, { path });
      await sync();
    } catch (reason: unknown) {
      setError(String(reason));
    }
  }, [sync]);

  /** Opens a folder: added to the top of the list and made current. */
  const open = useCallback((path: string) => run("workspace_open", path), [run]);
  /** Points the tools at a folder that is already open. */
  const select = useCallback((path: string) => run("workspace_select", path), [run]);
  /** Drops a folder. If it was current, the root moves to what is left. */
  const close = useCallback((path: string) => run("workspace_close", path), [run]);

  return { workspaces: state, error, open, select, close };
}
