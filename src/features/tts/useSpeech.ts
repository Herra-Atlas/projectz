import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import { playPcm, stopPlayback } from "./player";
import type { VoiceInfo } from "./types";

/** What a message's read-aloud button needs to know. */
export type Speech = {
  /** Null until the backend has answered. */
  voice: VoiceInfo | null;
  /** The key of the message being read, or null when nothing is. */
  speaking: string | null;
  /** Starts reading, or stops if `key` is already being read. */
  toggle: (key: string, text: string) => void;
  stop: () => void;
};

/**
 * Reads messages aloud through Piper.
 *
 * # One request at a time
 *
 * `piper.exe` is a process and synthesis is the slow half, so a user pressing
 * the button twice would otherwise spawn two processes and hear the second voice
 * over the first. `busy` covers that wait; once audio is playing a press on
 * another message replaces it, because the player stops what it is given.
 *
 * The voice is asked for once. It cannot change while the app is open -- there is
 * no voice picker yet -- and a failed read is reported where the button is, not
 * here.
 */
export function useSpeech(onError: (message: string) => void): Speech {
  const [voice, setVoice] = useState<VoiceInfo | null>(null);
  const [speaking, setSpeaking] = useState<string | null>(null);
  // A ref, not state: the guard is read inside an async callback that must see
  // the current value, not the one from the render that created it.
  const busy = useRef(false);
  const activeKey = useRef<string | null>(null);

  useEffect(() => {
    let live = true;
    invoke<VoiceInfo>("tts_voice")
      .then((next) => {
        if (live) setVoice(next);
      })
      .catch(() => {
        // Unreachable in practice -- the command answers with `available: false`
        // rather than failing -- so a read failure only means the button stays
        // disabled until the next open.
      });
    return () => {
      live = false;
    };
  }, []);

  const stop = useCallback(() => {
    stopPlayback();
    activeKey.current = null;
    setSpeaking(null);
  }, []);

  // Leaving the chat stops the audio. A voice still talking over the previous
  // conversation is the one thing this button must never cause.
  useEffect(() => stop, [stop]);

  const toggle = useCallback(
    (key: string, text: string) => {
      if (activeKey.current === key) {
        stop();
        return;
      }
      if (busy.current) return;

      busy.current = true;
      activeKey.current = key;
      setSpeaking(key);

      void (async () => {
        try {
          // Asked again here rather than trusted from mount: the voice can be
          // changed in Settings while this page stays open, and the rate belongs
          // to the voice. Two cheap round trips beat one wrong pitch.
          const current = await invoke<VoiceInfo>("tts_voice");
          if (!current.available) {
            throw new Error(current.reason ?? "No voice is installed.");
          }
          const bytes = await invoke<ArrayBuffer>("tts_speak", { text });
          // An audio.cpp voice answers `0` until its first synthesis: the rate
          // only exists once the engine has spoken. The synthesis above is that
          // first synthesis, so one re-read lands the true figure — and for
          // Piper the re-read is skipped, because nothing changed.
          const rate =
            current.sampleRate > 0
              ? current.sampleRate
              : (await invoke<VoiceInfo>("tts_voice")).sampleRate;
          playPcm(bytes, rate, () => {
            activeKey.current = null;
            setSpeaking(null);
          });
        } catch (error) {
          activeKey.current = null;
          setSpeaking(null);
          onError(error instanceof Error ? error.message : String(error));
        } finally {
          busy.current = false;
        }
      })();
    },
    [onError, stop, voice],
  );

  return { voice, speaking, toggle, stop };
}
