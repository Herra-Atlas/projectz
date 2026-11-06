import { invoke } from "@tauri-apps/api/core";

import type { SttModel } from "./types";

/**
 * The recognition model list, read once and shared by every page that draws it.
 *
 * The page used to call `stt_models` on every mount and paint `[]` until the
 * answer arrived -- which made "No audio.cpp model yet" flash over a library that
 * was only still being read. Two changes fix that, and they are the two halves of
 * this module: the list survives a page switch, and a page drawing it before the
 * first answer knows the difference between "empty" and "not read yet"
 * (`cachedSttModels()` returns `null` for the latter).
 *
 * This is the same shape as `features/settings/generalSettings.ts`: one cached
 * value, one in-flight request shared by concurrent callers, and subscribers told
 * when a fresh answer lands. A failure is never cached, so a read that lost a
 * race with startup is tried again rather than leaving an empty library behind
 * for the rest of the run.
 */

let cached: SttModel[] | null = null;
let pending: Promise<SttModel[]> | null = null;
const listeners = new Set<(models: SttModel[]) => void>();

/** The list if it has been read this run, otherwise `null`. */
export function cachedSttModels(): SttModel[] | null {
  return cached;
}

/** Listens for a fresh list; returns the unsubscribe a hook effect needs. */
export function subscribeSttModels(
  listener: (models: SttModel[]) => void,
): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function publish(models: SttModel[]) {
  cached = models;
  for (const listener of listeners) listener(models);
}

/**
 * The model list, from the cache when there is one.
 *
 * `force` re-reads after something changed on disk -- a download landed, a model
 * was deleted -- which is the only two reasons a cached list goes stale.
 */
export function loadSttModels(force = false): Promise<SttModel[]> {
  if (!force && cached) return Promise.resolve(cached);
  if (pending) return pending;
  pending = invoke<SttModel[]>("stt_models").then(
    (models) => {
      pending = null;
      publish(models);
      return models;
    },
    (error: unknown) => {
      pending = null;
      throw error;
    },
  );
  return pending;
}
