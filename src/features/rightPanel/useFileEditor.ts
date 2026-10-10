import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

/** Saving settles this long after the last keystroke, so one pause is one write. */
const AUTOSAVE_DELAY = 800;

export type SaveState = "clean" | "dirty" | "saving" | "error";

/**
 * The file's editable text with a debounced write-through to disk.
 *
 * One hook rather than logic inside `FilePreview` because the save lifecycle --
 * load, debounce, flush on unmount, Ctrl+S -- is a concern beside rendering.
 * The text always renders from `text`; the save only reads it.
 *
 * `enabled` is false for a file with no text: a picture, a PDF, a Word document.
 * The hook is still called -- it is a hook, so it cannot be skipped -- but it
 * reads nothing and writes nothing, which is what keeps a `.docx` from failing
 * with a UTF-8 error on its way to being drawn as a document.
 */
export function useFileEditor(path: string, enabled = true) {
  const [text, setText] = useState<string | undefined>(undefined);
  const [savedText, setSavedText] = useState<string | null>(null);
  const [state, setState] = useState<SaveState>("clean");
  const [error, setError] = useState<string | null>(null);

  const latestRef = useRef<string | null>(null);
  const savedRef = useRef<string | null>(null);
  const timerRef = useRef<number | null>(null);
  /** Read inside `flush`, which is assigned once per render but called later. */
  const enabledRef = useRef(enabled);
  enabledRef.current = enabled;

  latestRef.current = text ?? null;

  const clearTimer = useCallback(() => {
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  }, []);

  const saveNow = useCallback(async (value: string) => {
    setState("saving");
    setError(null);
    try {
      await invoke("panel_write_file", { relative: path, content: value });
      savedRef.current = value;
      setSavedText(value);
      setState("clean");
    } catch (reason) {
      setError(String(reason));
      setState("error");
    }
  }, [path]);

  const flushRef = useRef<() => void>(() => undefined);
  flushRef.current = () => {
    if (!enabledRef.current) return;
    const latest = latestRef.current;
    const saved = savedRef.current;
    if (latest === null || saved === null || latest === saved) return;
    clearTimer();
    void saveNow(latest);
  };

  const set = useCallback((value: string) => {
    setText(value);
    setState((current) => (value === savedRef.current ? (current === "saving" ? current : "clean") : "dirty"));
    setError(null);
    if (timerRef.current !== null) window.clearTimeout(timerRef.current);
    timerRef.current = window.setTimeout(() => {
      timerRef.current = null;
      flushRef.current();
    }, AUTOSAVE_DELAY);
  }, []);

  const retry = useCallback(() => {
    const latest = latestRef.current;
    if (latest === null) return;
    clearTimer();
    void saveNow(latest);
  }, [clearTimer, saveNow]);

  useEffect(() => {
    let active = true;
    setText(undefined);
    setSavedText(null);
    savedRef.current = null;
    latestRef.current = null;
    setState("clean");
    setError(null);
    clearTimer();
    // Cleared before the early return, so switching from a text file to a
    // picture does not leave the old file's text on screen behind the preview.
    if (!enabled) return;
    invoke<string>("panel_read_preview", { relative: path })
      .then((value) => {
        if (!active) return;
        savedRef.current = value;
        setSavedText(value);
        setText(value);
      })
      .catch((reason: unknown) => {
        if (!active) return;
        setError(String(reason));
        setState("error");
      });
    return () => {
      active = false;
      flushRef.current();
      clearTimer();
    };
  }, [path, enabled, clearTimer]);

  return { text, savedText, state, error, set, retry, saveNow: () => flushRef.current() };
}
