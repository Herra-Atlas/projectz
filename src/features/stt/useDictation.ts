import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/** What the microphone is doing, in the order it does it. */
export type DictationState = "idle" | "recording" | "transcribing";

export type Dictation = {
  state: DictationState;
  /** Starts a recording on a press, ends and transcribes it on the next. */
  toggle: () => void;
  /**
   * Begins recording, unless one is already running or the last one is still
   * being transcribed. Used by the wake word, which presses the button for the
   * user; a toggle there would stop a recording someone had already started.
   */
  start: () => void;
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
  /**
   * Called instead of `onText` when the user has asked transcripts to send
   * themselves. One or the other, never both: appending to the draft and then
   * sending it would put the words in the field and in the message.
   */
  onCommit?: (text: string) => void,
): Dictation {
  const [state, setState] = useState<DictationState>("idle");
  // Guards a double press while the first command is still in flight, which would
  // otherwise open two streams on one device.
  const busy = useRef(false);
  const recording = useRef(false);
  recording.current = state === "recording";
  // Read at the start of each recording rather than held in state: the setting
  // lives in Settings, and a press is the moment it has to be current.
  const autoSend = useRef(false);

  // One finish, for the press and for the backend ending a recording by itself:
  // the two paths have to leave the composer in exactly the same state.
  const finish = useCallback(() => {
    if (busy.current) return;
    busy.current = true;
    setState("transcribing");
    void invoke<string>("stt_recording_stop")
      .then((text) => {
        const trimmed = text.trim();
        if (!trimmed) return;
        if (autoSend.current && onCommit) onCommit(trimmed);
        else onText(trimmed);
      })
      .catch((reason: unknown) => onError(String(reason)))
      .finally(() => {
        busy.current = false;
        setState("idle");
      });
  }, [onText, onError, onCommit]);

  const toggle = useCallback(() => {
    if (busy.current) return;

    if (state === "recording") {
      finish();
      return;
    }
    if (state === "transcribing") return;

    busy.current = true;
    void invoke<{ autoSend?: boolean }>("stt_settings")
      .then((settings) => {
        autoSend.current = settings.autoSend === true;
      })
      .catch(() => undefined);
    void invoke("stt_recording_start")
      .then(() => setState("recording"))
      .catch((reason: unknown) => onError(String(reason)))
      .finally(() => {
        busy.current = false;
      });
  }, [state, finish, onError]);

  // Leaving the page must not leave the device open: the stream is owned by the
  // backend, so it is closed there rather than by dropping a handle here.
  useEffect(
    () => () => {
      if (!recording.current) return;
      void invoke("stt_recording_stop").catch(() => undefined);
    },
    [],
  );

  // A recording ends itself once the speech stops -- which is the only thing
  // that makes a wake word usable, because nobody is going to press the button
  // a second time. The listener is subscribed once; the ref is how it sees the
  // current state and the current callbacks.
  const finishRef = useRef(finish);
  finishRef.current = finish;
  useEffect(() => {
    let live = true;
    let stop: (() => void) | undefined;
    void listen("stt-recording-auto-stopped", () => {
      if (!live || !recording.current) return;
      finishRef.current();
    }).then((unlisten) => {
      if (live) stop = unlisten;
      else unlisten();
    });
    return () => {
      live = false;
      stop?.();
    };
  }, []);

  const start = useCallback(() => {
    if (state !== "idle") return;
    toggle();
  }, [state, toggle]);

  return { state, toggle, start };
}
