import { invoke } from "@tauri-apps/api/core";

import type { VoiceEntry } from "./types";

/**
 * The voice list, read once and shared by every page that draws it.
 *
 * Same problem and same shape as `features/stt/modelList.ts`: the synthesis page
 * re-read the whole library on every mount, and a list read from folders plus
 * audio.cpp's config is not instant with a large library. The cache makes a
 * second visit draw its rows on the first frame, and `null` until then keeps an
 * empty state from claiming there are no voices when the answer is merely late.
 */

let cached: VoiceEntry[] | null = null;
let pending: Promise<VoiceEntry[]> | null = null;
const listeners = new Set<(voices: VoiceEntry[]) => void>();

/** The list if it has been read this run, otherwise `null`. */
export function cachedTtsVoices(): VoiceEntry[] | null {
  return cached;
}

/** Listens for a fresh list; returns the unsubscribe a hook effect needs. */
export function subscribeTtsVoices(
  listener: (voices: VoiceEntry[]) => void,
): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function publish(voices: VoiceEntry[]) {
  cached = voices;
  for (const listener of listeners) listener(voices);
}

/**
 * The voice list, from the cache when there is one.
 *
 * `force` re-reads after a voice was installed or deleted, or after a speech
 * model loaded and became the selected voice.
 */
export function loadTtsVoices(force = false): Promise<VoiceEntry[]> {
  if (!force && cached) return Promise.resolve(cached);
  if (pending) return pending;
  pending = invoke<VoiceEntry[]>("tts_voices").then(
    (voices) => {
      pending = null;
      publish(voices);
      return voices;
    },
    (error: unknown) => {
      pending = null;
      throw error;
    },
  );
  return pending;
}
