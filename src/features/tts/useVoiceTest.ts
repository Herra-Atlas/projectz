import { useCallback, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import { playPcm } from "./player";
import type { VoiceInfo } from "./types";

/** What a test says. Short on purpose: it is a pace check, not a demo. */
const SAMPLE = "This is how I sound.";

/**
 * Speaks one short sentence in a named voice.
 *
 * The voice is named explicitly rather than taken from the selection, so a
 * settings page can let a reader hear a voice before choosing it. The rate comes
 * from the same command, because it belongs to the voice -- and hearing a voice
 * at another voice's rate is exactly the wrong test.
 */
export function useVoiceTest(onError: (message: string) => void) {
  const [testing, setTesting] = useState(false);

  const test = useCallback(
    async (voice: string | null) => {
      // One at a time: `piper.exe` is a process, and a held-down button would
      // spawn one per press.
      if (testing) return;
      setTesting(true);
      try {
        const info = await invoke<VoiceInfo>("tts_voice", { voice });
        if (!info.available) {
          throw new Error(info.reason ?? "No voice is installed.");
        }
        const bytes = await invoke<ArrayBuffer>("tts_speak", { text: SAMPLE, voice });
        playPcm(bytes, info.sampleRate, () => undefined);
      } catch (reason) {
        onError(reason instanceof Error ? reason.message : String(reason));
      } finally {
        setTesting(false);
      }
    },
    [onError, testing],
  );

  return { testing, test };
}
