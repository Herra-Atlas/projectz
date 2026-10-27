import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import type { VoiceStatus } from "./types";

/**
 * The Voice recognition pages' state.
 *
 * Every command answers with the whole status rather than a patch: both pages
 * draw one record, and reading it back separately would be a second thing to
 * keep in step. A failed action reports and re-reads, so a page never keeps a
 * state the backend refused.
 */
export function useVoices(onError: (message: string) => void) {
  const [status, setStatus] = useState<VoiceStatus | null>(null);

  const refresh = useCallback(() => {
    void invoke<VoiceStatus>("voice_status")
      .then(setStatus)
      .catch((reason: unknown) => onError(String(reason)));
  }, [onError]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const apply = useCallback(
    (command: string, args?: Record<string, unknown>, what = "That did not work") => {
      return invoke<VoiceStatus>(command, args)
        .then((next) => {
          setStatus(next);
          return next;
        })
        .catch((reason: unknown) => {
          onError(`${what}: ${String(reason)}`);
          refresh();
          return null;
        });
    },
    [onError, refresh],
  );

  return {
    status,
    refresh,
    setEnabled: (enabled: boolean) =>
      apply("voice_set_enabled", { enabled }, "Could not change that"),
    setThreshold: (threshold: number) =>
      apply("voice_set_threshold", { threshold }, "The sensitivity was not saved"),
    /** Records and embeds one take; resolves false when it was refused. */
    recordTake: () =>
      invoke<VoiceStatus>("voice_enroll_clip")
        .then((next) => {
          setStatus(next);
          return true;
        })
        .catch((reason: unknown) => {
          onError(`That take was not recorded: ${String(reason)}`);
          refresh();
          return false;
        }),
    saveVoice: (name: string) => apply("voice_enroll_finish", { name }, "The voice was not saved"),
    discard: () => apply("voice_enroll_cancel"),
    remove: (name: string) => apply("voice_delete", { name }, "The voice was not deleted"),
  };
}
