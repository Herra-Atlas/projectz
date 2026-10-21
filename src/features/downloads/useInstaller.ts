import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import { downloadKey, type DownloadFile, type DownloadKind, type DownloadProgress } from "./types";

/** Installing, and how far along each row is. */
export type Installer = {
  /** The key of the row currently being written, or null. */
  installing: string | null;
  /** Fraction complete, by row key. Absent while the size is unknown. */
  progress: Record<string, number>;
  /** Rows installed since the page opened. */
  installed: ReadonlySet<string>;
  install: (file: DownloadFile) => void;
};

/**
 * Downloads one row at a time.
 *
 * # Why one at a time
 *
 * A model is gigabytes. Two at once do not finish sooner -- they share the same
 * link -- and a second start while the first is writing is almost always a
 * mis-click, so the button is disabled rather than queued.
 *
 * Progress arrives as an event from the Rust side, which emits it while the
 * body is being streamed; nothing here polls.
 */
export function useInstaller(
  kind: DownloadKind,
  onInstalled: (file: DownloadFile, installedPath: string) => void,
  onError: (message: string) => void,
): Installer {
  const [installing, setInstalling] = useState<string | null>(null);
  const [progress, setProgress] = useState<Record<string, number>>({});
  const [installed, setInstalled] = useState<ReadonlySet<string>>(new Set());

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let live = true;
    void listen<DownloadProgress>("downloads-progress", (event) => {
      const { repo, path, received, total } = event.payload;
      if (!total) return;
      setProgress((current) => ({ ...current, [`${repo}/${path}`]: received / total }));
    }).then((stop) => {
      if (live) unlisten = stop;
      else stop();
    });
    return () => {
      live = false;
      unlisten?.();
    };
  }, []);

  const install = useCallback(
    (file: DownloadFile) => {
      if (installing) return;
      const key = downloadKey(file);
      setInstalling(key);
      setProgress((current) => ({ ...current, [key]: 0 }));

      void invoke<string>("downloads_install", { kind, repo: file.repo, path: file.path })
        .then((installedPath) => {
          setInstalled((current) => new Set(current).add(key));
          // The path comes back from the installer, which is the side that knows
          // the layout; deriving it here would be a second copy of that rule.
          onInstalled(file, installedPath);
        })
        .catch((reason: unknown) => onError(String(reason)))
        .finally(() => {
          setInstalling(null);
          // The bar has done its job; leaving it at 100% beside an "Installed"
          // label says the same thing twice.
          setProgress((current) => {
            const next = { ...current };
            delete next[key];
            return next;
          });
        });
    },
    [installing, kind, onError, onInstalled],
  );

  // Memoised so a caller can pass it as a dependency without re-running effects.
  return useMemo(
    () => ({ installing, progress, installed, install }),
    [install, installed, installing, progress],
  );
}
