import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import { playPcm, stopPlayback, unlockAudio } from "./player";
import type { TtsSettings, VoiceInfo } from "./types";

/** What the read-aloud control needs to know. */
export type SpeechPhase = "synthesizing" | "playing";
export type Speech = {
  /** Null until the backend has answered. */
  voice: VoiceInfo | null;
  /** The key of the message being spoken, or null when nothing is. */
  speaking: string | null;
  /**
   * `synthesizing` until the first sound starts, `playing` from then on.
   *
   * Two states rather than one because they are different waits: the button says
   * "Preparing" while the first sentence is being made, and only then becomes a
   * Stop with the equalizer. A suspended `AudioContext` used to leave this at
   * "speaking" for an answer nobody could hear.
   */
  phase: SpeechPhase | null;
  /** Starts reading, or stops if `key` is already being read. */
  toggle: (key: string, text: string) => void;
  /** Stops the voice at once. */
  stop: () => void;
  /**
   * Re-asks the backend whether there is a voice to read with.
   *
   * The chat page is mounted for the life of the app while Settings is a
   * sibling, so a voice chosen after the page opened would otherwise leave the
   * Read button dim until a restart. This is the one fact that must follow the
   * settings record rather than the page's mount.
   */
  refresh: () => Promise<void>;
};

/**
 * How many sentences are synthesized ahead of the one being heard.
 *
 * Two ahead means three in flight at most: the sentence playing, the next one
 * ready, and a third being made. That is what makes a hundred-sentence reply
 * start as fast as a one-sentence reply -- the wait is one sentence, not the
 * whole message -- while a single local engine is never asked for more work
 * than it can finish before the previous sentence stops.
 */
const PREFETCH = 2;

/** A fragment shorter than this is joined to its neighbour rather than queued. */
const MIN_CHUNK = 40;

/** Above this a sentence is split again at the last natural break. */
const MAX_CHUNK = 240;

/**
 * Splits cleaned speech into pieces worth one synthesis each.
 *
 * Sentence endings first, then two corrections: a very short piece ("Yes.",
 * "Right.") is joined to the previous one so the engine is not started for a
 * word, and an overlong piece is broken at a comma, semicolon or space so no
 * single request waits on a paragraph. A piece with no letters or digits at all
 * is dropped -- punctuation alone has nothing to say.
 */
export function splitSpeech(text: string): string[] {
  const sentences: string[] = [];
  let start = 0;
  for (let index = 0; index < text.length; index += 1) {
    const character = text[index];
    const endsSentence =
      (character === "." || character === "!" || character === "?") &&
      (index + 1 === text.length || /\s/.test(text[index + 1]));
    if (!endsSentence) continue;
    const piece = text.slice(start, index + 1).trim();
    if (piece) sentences.push(piece);
    start = index + 1;
  }
  const tail = text.slice(start).trim();
  if (tail) sentences.push(tail);

  const merged: string[] = [];
  for (const sentence of sentences) {
    const previous = merged[merged.length - 1];
    const short = sentence.length < MIN_CHUNK || (previous !== undefined && previous.length < MIN_CHUNK);
    if (previous !== undefined && short && previous.length + sentence.length + 1 <= MAX_CHUNK) {
      merged[merged.length - 1] = `${previous} ${sentence}`;
    } else {
      merged.push(sentence);
    }
  }

  return merged
    .flatMap((chunk) => (chunk.length <= MAX_CHUNK ? [chunk] : splitLong(chunk)))
    .filter((chunk) => /[A-Za-z0-9]/.test(chunk));
}

/** One overlong sentence, cut at the last break before the limit. */
function splitLong(chunk: string): string[] {
  const parts: string[] = [];
  let rest = chunk;
  while (rest.length > MAX_CHUNK) {
    let cut = -1;
    for (let index = MAX_CHUNK; index > MAX_CHUNK / 2; index -= 1) {
      if (",;: ".includes(rest[index])) {
        cut = index;
        break;
      }
    }
    if (cut <= 0) cut = rest.lastIndexOf(" ", MAX_CHUNK);
    if (cut <= 0) cut = MAX_CHUNK;
    // Never cut between the two halves of a surrogate pair: a chopped emoji is a
    // replacement character, which the engine pronounces as nothing at best.
    const code = rest.charCodeAt(cut);
    if (cut > 0 && code >= 0xd800 && code <= 0xdbff) cut -= 1;
    parts.push(rest.slice(0, cut + 1).trim());
    rest = rest.slice(cut + 1).trim();
  }
  if (rest) parts.push(rest);
  return parts;
}

/**
 * Reads messages aloud.
 *
 * # One voice at a time
 *
 * A playback owns a generation number. `stop`, a new message, and unmount all
 * move it on, so a synthesis that answers late can neither start audio nor
 * clear the state of the reply that replaced it.
 *
 * # Sentence by sentence
 *
 * With streaming on, the cleaned reply is split and synthesized with
 * [`PREFETCH`] requests in flight while the current piece plays. A long reply
 * starts speaking after one sentence and stays ahead of the ear; the previous
 * single-request design made the reader wait for the entire answer before the
 * first word. Off, the whole script is one request, exactly as it used to be.
 */
export function useSpeech(onError: (message: string) => void): Speech {
  const [voice, setVoice] = useState<VoiceInfo | null>(null);
  const [speaking, setSpeaking] = useState<string | null>(null);
  const [phase, setPhase] = useState<SpeechPhase | null>(null);
  // A ref, not state: these are read inside async callbacks that must see the
  // current value, not the one from the render that created them.
  const activeKey = useRef<string | null>(null);
  const generation = useRef(0);
  const busy = useRef(false);

  const refresh = useCallback(async () => {
    try {
      setVoice(await invoke<VoiceInfo>("tts_voice"));
    } catch {
      // Unreachable in practice -- the command answers with `available: false`
      // rather than failing -- so a read failure only means the button keeps its
      // previous answer until the next change.
    }
  }, []);

  // The voice is a fact about the settings record, not about this mount: a voice
  // chosen, downloaded or deleted while the chat is open has to reach the Read
  // button without a restart. Focus covers a change made outside the app (a
  // voice folder copied in), the two events cover every in-app door, and the
  // mount read seeds the first paint.
  useEffect(() => {
    void refresh();
    let live = true;
    const stops: Array<() => void> = [];
    const track = (stop: () => void) => {
      if (live) stops.push(stop);
      else stop();
    };
    void listen("tts-voice-changed", () => void refresh()).then(track);
    void listen("downloads-installed", () => void refresh()).then(track);
    const onFocus = () => void refresh();
    window.addEventListener("focus", onFocus);
    return () => {
      live = false;
      stops.forEach((stop) => stop());
      window.removeEventListener("focus", onFocus);
    };
  }, [refresh]);

  /**
   * Opens the output device on the first gesture.
   *
   * Auto-speak runs when a reply finishes, which is not a gesture. If nobody has
   * pressed anything yet the context stays suspended, so the automatic read is
   * silent -- and so is the next one, because a suspended context never ends the
   * source it was given. The first press or key in the chat is enough, and it
   * costs one listener while the page is open.
   */
  useEffect(() => {
    const unlock = () => unlockAudio();
    const events = ["pointerdown", "keydown"] as const;
    for (const event of events) {
      window.addEventListener(event, unlock, { once: true, capture: true });
    }
    return () => {
      for (const event of events) {
        window.removeEventListener(event, unlock, { capture: true });
      }
    };
  }, []);

  /** Ends the current read, whoever started it. */
  const stop = useCallback(() => {
    generation.current += 1;
    busy.current = false;
    activeKey.current = null;
    stopPlayback();
    setSpeaking(null);
    setPhase(null);
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
      // A press on another message replaces what is being read, including a
      // synthesis still in flight. The old read's generation is retired so its
      // answer cannot start playing over the new one.
      if (busy.current) stop();

      const token = generation.current + 1;
      generation.current = token;
      busy.current = true;
      activeKey.current = key;
      setSpeaking(key);
      setPhase("synthesizing");

      void (async () => {
        try {
          const current = await invoke<VoiceInfo>("tts_voice");
          if (token !== generation.current) return;
          if (!current.available) throw new Error(current.reason ?? "No voice is installed.");

          const settings = await invoke<TtsSettings>("tts_settings").catch(() => null);
          if (token !== generation.current) return;

          // Cleaned once, for the whole reply: the fences and markup rules need
          // the document, not a sentence of it.
          const script = await invoke<string>("tts_clean", { text });
          if (token !== generation.current) return;
          if (!script.trim()) throw new Error("There is nothing to read under the current skip settings.");

          let rate = current.sampleRate;
          const chunks = settings?.streamSpeech === false ? [script] : splitSpeech(script);
          if (chunks.length === 0) throw new Error("There is nothing to read under the current skip settings.");

          const synthesize = (chunk: string) =>
            invoke<ArrayBuffer>("tts_speak", { text: chunk, prepared: true });

          // A small rolling window: fill it, play the oldest, fill the gap.
          const pending: Promise<ArrayBuffer>[] = [];
          let queued = 0;
          const fill = () => {
            while (pending.length < PREFETCH && queued < chunks.length) {
              const job = synthesize(chunks[queued]);
              // A prefetched request can fail before the queue reaches it; the
              // await below still sees the rejection, but without this handler
              // the browser reports it as unhandled in between.
              void job.catch(() => undefined);
              pending.push(job);
              queued += 1;
            }
          };
          fill();

          for (let index = 0; index < chunks.length; index += 1) {
            const bytes = await pending.shift();
            fill();
            if (token !== generation.current || !bytes) return;
            // audio.cpp reports `0` until its first synthesis, so the true rate
            // is only knowable now -- and the rate belongs to the voice, not the
            // sentence.
            if (rate <= 0) {
              rate = (await invoke<VoiceInfo>("tts_voice")).sampleRate;
              if (token !== generation.current) return;
            }
            setPhase("playing");
            await new Promise<void>((resolve) => playPcm(bytes, rate, resolve));
            if (token !== generation.current) return;
          }
        } catch (error) {
          if (token === generation.current) {
            onError(error instanceof Error ? error.message : String(error));
          }
        } finally {
          if (token === generation.current) {
            busy.current = false;
            activeKey.current = null;
            setSpeaking(null);
            setPhase(null);
          }
        }
      })();
    },
    [onError, stop],
  );

  return { voice, speaking, phase, toggle, stop, refresh };
}
