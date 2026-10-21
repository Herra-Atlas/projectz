import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

/** What the microphone is doing, in the order it does it. */
export type DictationState = "idle" | "recording" | "transcribing";

export type Dictation = {
  state: DictationState;
  /** Starts a recording on a press, ends and transcribes it on the next. */
  toggle: () => void;
};

/**
 * The composer's microphone.
 *
 * Push to talk rather than always listening: the device is open only between the
 * two presses, so nothing records in the background and no battery is spent
 * deciding whether silence contains a word. The engine is a process, so the
 * transcription state is visible instead of the button appearing to do nothing.
 */
export function useDictation(
  onText: (text: string) => void,
  onError: (message: string) => void,
): Dictation {
  const [state, setState] = useState<DictationState>("idle");
  // Guards a double press while the first command is still in flight, which would
  // otherwise open two streams on one device.
  const busy = useRef(false);
  const recording = useRef(false);
  recording.current = state === "recording";

  const toggle = useCallback(() => {
    if (busy.current) return;
    busy.current = true;

    if (state === "recording") {
      setState("transcribing");
      void invoke<string>("stt_recording_stop")
        .then((text) => {
          const trimmed = text.trim();
          if (trimmed) onText(trimmed);
        })
        .catch((reason: unknown) => onError(String(reason)))
        .finally(() => {
          busy.current = false;
          setState("idle");
        });
      return;
    }

    if (state === "transcribing") {
      busy.current = false;
      return;
    }

    void invoke("stt_recording_start")
      .then(() => setState("recording"))
      .catch((reason: unknown) => onError(String(reason)))
      .finally(() => {
        busy.current = false;
      });
  }, [state, onText, onError]);

  // Leaving the page must not leave the device open: the stream is owned by the
  // backend, so it is closed there rather than by dropping a handle here.
  useEffect(
    () => () => {
      if (!recording.current) return;
      void invoke("stt_recording_stop").catch(() => undefined);
    },
    [],
  );

  return { state, toggle };
}
