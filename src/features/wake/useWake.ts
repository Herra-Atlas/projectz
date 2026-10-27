import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { WakeStatus } from "./types";

/** The Wake word page's state and the five things it can change. */
export type Wake = {
  status: WakeStatus | null;
  refresh: () => Promise<void>;
  setEnabled: (enabled: boolean) => Promise<void>;
  selectModel: (model: string) => Promise<void>;
  setThreshold: (threshold: number) => Promise<void>;
  addModel: (source: string) => Promise<void>;
  removeModel: (model: string) => Promise<void>;
};

/**
 * The wake word's record, and the calls that change it.
 *
 * Every command answers with the whole status rather than nothing, so the page
 * never has to guess what a failed start left behind: the switch it draws is
 * the record the backend holds, not a copy of local state.
 */
export function useWake(onError: (message: string) => void): Wake {
  const [status, setStatus] = useState<WakeStatus | null>(null);

  const refresh = useCallback(async () => {
    try {
      setStatus(await invoke<WakeStatus>("wake_status"));
    } catch (reason) {
      onError(`Could not read the wake word settings: ${String(reason)}`);
    }
  }, [onError]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // A model installed on the download page is a sibling page: the only way this
  // list hears about it is the event the installer emits. Without this a freshly
  // downloaded word never appears, and saying it does nothing at all.
  useEffect(() => {
    let live = true;
    let stop: (() => void) | undefined;
    void listen("downloads-installed", () => void refresh()).then((unlisten) => {
      if (live) stop = unlisten;
      else unlisten();
    });
    return () => {
      live = false;
      stop?.();
    };
  }, [refresh]);

  const apply = useCallback(
    async (command: string, args: Record<string, unknown>, failure: string) => {
      try {
        setStatus(await invoke<WakeStatus>(command, args));
      } catch (reason) {
        onError(`${failure}: ${String(reason)}`);
        // The record is the truth even when the change was refused.
        await refresh();
      }
    },
    [onError, refresh],
  );

  const setEnabled = useCallback(
    (enabled: boolean) =>
      apply("wake_set_enabled", { enabled }, enabled ? "Could not start listening" : "Could not stop listening"),
    [apply],
  );

  const selectModel = useCallback(
    (model: string) => apply("wake_select_model", { model }, "That model could not be chosen"),
    [apply],
  );

  const setThreshold = useCallback(
    (threshold: number) => apply("wake_set_threshold", { threshold }, "The threshold could not be saved"),
    [apply],
  );

  const addModel = useCallback(
    (source: string) => apply("wake_add_model", { source }, "That model could not be added"),
    [apply],
  );

  const removeModel = useCallback(
    (model: string) => apply("wake_remove_model", { model }, "That model could not be deleted"),
    [apply],
  );

  return { status, refresh, setEnabled, selectModel, setThreshold, addModel, removeModel };
}
