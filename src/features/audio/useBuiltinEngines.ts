import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";

import type { BuiltinEngine } from "./useAudioEngine";

/** One engine's install fraction while its archive arrives. */
type EngineProgress = { id: string; downloaded: number; total: number };

/** What a page that only asks "is the engine installed?" needs. */
export type BuiltinEngines = {
  /** Null until the first read answers. */
  engines: BuiltinEngine[] | null;
  installed: (id: string) => boolean;
  /** The engine being installed, and its fraction, for the button's spinner. */
  installing: string | null;
  progress: number | null;
  install: (id: string) => void;
  refresh: () => Promise<void>;
};

/**
 * The built-in engines' install state, without the Audio page's remote catalog.
 *
 * `audio_engines_builtin` is a folder probe for every engine, which is all a
 * synthesis page needs to choose between "Install engine" and "Download voice".
 * The release catalog is a network read with no bearing on that question, so it
 * stays on the page that draws it.
 */
export function useBuiltinEngines(onError: (message: string) => void): BuiltinEngines {
  const [engines, setEngines] = useState<BuiltinEngine[] | null>(null);
  const [installing, setInstalling] = useState<string | null>(null);
  const [progress, setProgress] = useState<number | null>(null);
  // Held rather than depended on: an inline arrow from the caller would rebuild
  // `refresh` every render and re-fire the mount read.
  const report = useRef(onError);
  report.current = onError;

  const refresh = useCallback(async () => {
    try {
      setEngines(await invoke<BuiltinEngine[]>("audio_engines_builtin"));
    } catch (reason) {
      report.current(`Could not read the engines: ${String(reason)}`);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // An install can finish from another page (the Audio engine page is a
  // sibling), and a folder can appear while this page is open; both are the
  // same fact, so both re-read rather than trusting the mount-time answer.
  useEffect(() => {
    let live = true;
    const stops: Array<() => void> = [];
    const track = (stop: () => void) => {
      if (live) stops.push(stop);
      else stop();
    };
    void listen<EngineProgress>("engine-download-progress", (event) => {
      const { id, downloaded, total } = event.payload;
      if (id !== "piper" && id !== "whisper" && !id.startsWith("audiocpp-")) return;
      setProgress(total > 0 ? downloaded / total : null);
    }).then(track);
    void listen("downloads-installed", () => void refresh()).then(track);
    return () => {
      live = false;
      stops.forEach((stop) => stop());
    };
  }, [refresh]);

  const install = useCallback(
    (id: string) => {
      setInstalling(id);
      setProgress(0);
      void invoke("audio_builtin_install", { id })
        .then(() =>
          // Installing Piper is exactly what makes a Piper voice readable, so
          // the chat's Read button has to hear about it in the same breath. A
          // failed broadcast must not turn a finished install into an error.
          refresh().then(() => {
            void emit("tts-voice-changed").catch(() => undefined);
          }),
        )
        .catch((reason: unknown) =>
          report.current(`The engine could not be installed: ${String(reason)}`),
        )
        .finally(() => {
          setInstalling(null);
          setProgress(null);
        });
    },
    [refresh],
  );

  const installed = useCallback(
    (id: string) => engines?.find((engine) => engine.id === id)?.installed === true,
    [engines],
  );

  return { engines, installed, installing, progress, install, refresh };
}
