import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import type { DownloadFile, DownloadKind } from "./types";

/** What one search settled on, and whether it is still running. */
export type DownloadSearch = {
  files: DownloadFile[];
  loading: boolean;
  error: string | null;
};

/** How long typing has to stop before a search is sent. */
const QUIET_MS = 300;

/**
 * Files installable for `kind`, matching `query`.
 *
 * # Why it waits for quiet
 *
 * A llama.cpp search is one request plus up to thirty for the repositories'
 * file lists. Firing one per keystroke would spend that on every letter of a
 * word the user has not finished typing, and the answers would arrive in an
 * order nobody asked for.
 *
 * The previous search's answer is dropped rather than merged: `live` is per
 * attempt, so a slow response cannot overwrite the result of a newer query.
 */
export function useDownloadSearch(kind: DownloadKind, query: string): DownloadSearch {
  const [state, setState] = useState<DownloadSearch>({ files: [], loading: true, error: null });

  useEffect(() => {
    let live = true;
    setState((current) => ({ ...current, loading: true, error: null }));

    const timer = window.setTimeout(() => {
      invoke<DownloadFile[]>("downloads_search", { kind, query })
        .then((files) => {
          if (live) setState({ files, loading: false, error: null });
        })
        .catch((reason: unknown) => {
          if (live) setState({ files: [], loading: false, error: String(reason) });
        });
    }, QUIET_MS);

    return () => {
      live = false;
      window.clearTimeout(timer);
    };
  }, [kind, query]);

  return state;
}
