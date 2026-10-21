import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { SttProgress, SttStatus } from "./types";

/** The recognition row's state, and the one action it offers. */
export type Stt = {
  status: SttStatus | null;
  /** Fraction of the engine download, or null when nothing is arriving. */
  progress: number | null;
  installing: boolean;
  install: () => void;
  refresh: () => Promise<void>;
};

/**
 * The engine: whether it is installed, and installing it.
 *
 * One download, one button. Progress is an event from the Rust side, which emits
 * it while the archive is streamed to disk and verified; nothing here polls.
 */
export function useStt(onError: (message: string) => void): Stt {
  const [status, setStatus] = useState<SttStatus | null>(null);
  const [progress, setProgress] = useState<number | null>(null);
  const [installing, setInstalling] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setStatus(await invoke<SttStatus>("stt_status"));
    } catch (reason) {
      onError(`Could not read the speech engine: ${String(reason)}`);
    }
  }, [onError]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let live = true;
    void listen<SttProgress>("stt-progress", (event) => {
      const { received, total } = event.payload;
      if (total) setProgress(received / total);
    }).then((stop) => {
      if (live) unlisten = stop;
      else stop();
    });
    return () => {
      live = false;
      unlisten?.();
    };
  }, []);

  const install = useCallback(() => {
    if (installing) return;
    setInstalling(true);
    setProgress(0);
    void invoke<SttStatus>("stt_install")
      .then((next) => setStatus(next))
      .catch((reason: unknown) => onError(`The speech engine could not be installed: ${String(reason)}`))
      .finally(() => {
        setInstalling(false);
        // The bar has done its job; leaving it full beside an "Installed" label
        // says the same thing twice.
        setProgress(null);
      });
  }, [installing, onError]);

  return { status, progress, installing, install, refresh };
}
