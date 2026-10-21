import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { DownloadJob, DownloadKind } from "./types";

/**
 * The app's downloads, as the backend sees them.
 *
 * The list lives in Rust, so it survives the page being closed, the settings
 * dialog being closed, and anything else the frontend does. This hook is a view
 * of it plus the four actions a row needs; it never holds progress of its own,
 * because two views of one download can disagree.
 */
export function useDownloads(onError?: (message: string) => void) {
  const [jobs, setJobs] = useState<DownloadJob[]>([]);

  const apply = useCallback((next: DownloadJob[]) => {
    setJobs(next);
  }, []);

  useEffect(() => {
    let live = true;
    let stop: (() => void) | undefined;
    void invoke<DownloadJob[]>("downloads_jobs")
      .then((next) => {
        if (live) apply(next);
      })
      .catch(() => undefined);
    void listen<DownloadJob[]>("downloads-state", (event) => apply(event.payload)).then(
      (unlisten) => {
        if (live) stop = unlisten;
        else unlisten();
      },
    );
    return () => {
      live = false;
      stop?.();
    };
  }, [apply]);

  const fail = useCallback(
    (reason: unknown, action: string) => {
      if (onError) onError(`${action}: ${String(reason)}`);
    },
    [onError],
  );

  const start = useCallback(
    (kind: DownloadKind, repo: string, path: string) => {
      void invoke("downloads_start", { kind, repo, path }).catch((reason: unknown) =>
        fail(reason, "The download could not be started"),
      );
    },
    [fail],
  );

  const pause = useCallback(
    (id: string) => {
      void invoke("downloads_pause", { id }).catch((reason: unknown) =>
        fail(reason, "The download could not be paused"),
      );
    },
    [fail],
  );

  const resume = useCallback(
    (id: string) => {
      void invoke("downloads_resume", { id }).catch((reason: unknown) =>
        fail(reason, "The download could not be resumed"),
      );
    },
    [fail],
  );

  const stop = useCallback(
    (id: string) => {
      void invoke("downloads_stop", { id }).catch((reason: unknown) =>
        fail(reason, "The download could not be stopped"),
      );
    },
    [fail],
  );

  return { jobs, start, pause, resume, stop };
}
