import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import type { AudioModelRow } from "./useAudioEngine";

/** One option a model's spec names, typed for the settings page. */
export type AudioOption = {
  /**
   * `request` — sent with every call; `load` and `session` — live in the
   * engine's config, so they need a server restart to take hold.
   */
  section: "request" | "load" | "session";
  name: string;
  kind: "text" | "int" | "float" | "bool" | "enum";
  description: string;
  required: boolean;
  default: null | string | number | boolean;
  values: string[];
};

/** What a model's settings page is built from. */
export type AudioModelSpec = {
  path: string;
  name: string;
  family: string;
  displayName: string | null;
  tasks: string[];
  task: "tts" | "asr" | "wake" | "unknown";
  capabilities: Record<string, string[]>;
  options: AudioOption[];
  values: Record<string, unknown>;
};

/** A model's spec, and the field edits the page is allowed to make. */
export type AudioModel = {
  model: AudioModelRow | null;
  spec: AudioModelSpec | null;
  /** Every current value, spec-declared or not -- the record the page saves. */
  values: Record<string, unknown>;
  /** Updates one stored value and saves it. */
  set: (name: string, value: unknown) => Promise<void>;
  /** Clears everything stored, so the spec's defaults answer again. */
  reset: () => Promise<void>;
};

/**
 * One model's page of options, read straight from its family's spec.
 *
 * The spec names what the model can take; the values are what the user has
 * set, and they ride per model -- a Parakeet keeps its language while a Kokoro
 * keeps its voice id, under one settings key rather than two opinions.
 */
export function useAudioModel(
  path: string,
  onModelGone: () => void,
  onError: (message: string) => void,
): AudioModel {
  const [model, setModel] = useState<AudioModelRow | null>(null);
  const [spec, setSpec] = useState<AudioModelSpec | null>(null);
  const [values, setValues] = useState<Record<string, unknown>>({});
  // Values are saved on change, so the copy the request was built from is what
  // the save against, not the render's snapshot.
  const valuesRef = useRef(values);

  // Held rather than depended on: both callbacks arrive as inline arrows at the
  // call site, and depending on them would give `load` a new identity every
  // render -- which re-read the model's spec off disk once per frame.
  const report = useRef(onError);
  report.current = onError;
  const gone = useRef(onModelGone);
  gone.current = onModelGone;

  const load = useCallback(async () => {
    try {
      const next = await invoke<AudioModelSpec>("audio_model_spec", { path });
      setSpec(next);
      setModel({ path: next.path, name: next.name, family: next.family, task: next.task, size: 0 });
      const stored = (next.values ?? {}) as Record<string, unknown>;
      setValues(stored);
      valuesRef.current = stored;
    } catch (reason) {
      // A stale path is the one failure the page cannot render around; the
      // message is what caused it, and going back is the way out.
      report.current(String(reason));
      gone.current();
    }
  }, [path]);

  useEffect(() => {
    void load();
  }, [load]);

  const set = useCallback(
    async (name: string, value: unknown) => {
      const next = { ...valuesRef.current, [name]: value };
      valuesRef.current = next;
      setValues(next);
      try {
        await invoke("audio_model_save_options", { path, values: next });
      } catch (reason) {
        report.current(`The setting could not be saved: ${String(reason)}`);
      }
    },
    [path],
  );

  const reset = useCallback(async () => {
    try {
      await invoke("audio_model_save_options", { path, values: {} });
      valuesRef.current = {};
      setValues({});
    } catch (reason) {
      report.current(`The settings could not be cleared: ${String(reason)}`);
    }
  }, [path]);

  return { model, spec, values, set, reset };
}
