import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/** What the audio pages state, one small record at a time. */

/** The three engines the app knows, as the engine page lists them. */
export type BuiltinEngine = {
  id: string;
  name: string;
  version: string;
  installed: boolean;
  /** How many voice or model files the engine serves. */
  assets: number;
  /** `catalog`, `fixed` -- who owns the files name and where the assets go. */
  kind: string;
};

/** One audio.cpp release the catalog lists. */
export type CatalogEngine = {
  id: string;
  version: string;
  updatedAt: string;
  assetName: string;
  sizeBytes: number;
  installed: boolean;
};

/** One installed audio.cpp model. */
export type AudioModelRow = {
  path: string;
  name: string;
  family: string;
  task: "tts" | "asr" | "unknown";
  size: number;
};

/** `audio_status`'s record. */
export type AudioStatus = {
  engineInstalled: boolean;
  engineVersion: string | null;
  models: number;
  running: boolean;
};

/** Whisper's install progress, on the event its own install has always used. */
export type SttProgress = { received: number; total: number | null };

/** One engine's install fraction while its archive arrives. */
type EngineProgress = { id: string; downloaded: number; total: number };

/** The Audio engine page's state, catalog included. */
export type AudioEngine = {
  builtins: BuiltinEngine[];
  catalog: CatalogEngine[];
  status: AudioStatus | null;
  installing: string | null;
  progress: number | null;
  installingWhisper: boolean;
  whisperProgress: number | null;
  /** Loads state. Called on mount, after an install and after a download lands. */
  refresh: () => Promise<void>;
  installCatalog: (id: string) => void;
  installBuiltin: (id: string) => void;
  uninstall: (id: string) => void;
  start: () => Promise<void>;
  stop: () => Promise<void>;
};

/**
 * What the last load answered, alive across page switches.
 *
 * Settings pages remount at every switch; stdin-fast as the backend calls are,
 * a hundred milliseconds of "Checking…" per visit is not worth its honesty.
 * The snapshot is painted on mount and replaced by the refresh, so the page
 * opens on what has been read before and shows the truth one frame later.
 */
let snapshot: {
  builtins: BuiltinEngine[];
  catalog: CatalogEngine[];
  status: AudioStatus | null;
} | null = null;

/**
 * Engine installs, all three from one page.
 *
 * audio.cpp progress rides the llama.cpp engines' event (ids tell whose bar is
 * whose); whisper keeps its older per-received event from the recognition page,
 * and Piper shares catalog's event under its own id. Any download landing
 * anywhere refreshes the whole page, which is what stops a row claiming an
 * engine the folder no longer has.
 */
export function useAudioEngine(onError: (message: string) => void): AudioEngine {
  const [builtins, setBuiltins] = useState<BuiltinEngine[]>(snapshot?.builtins ?? []);
  const [catalog, setCatalog] = useState<CatalogEngine[]>(snapshot?.catalog ?? []);
  const [status, setStatus] = useState<AudioStatus | null>(snapshot?.status ?? null);
  const [installing, setInstalling] = useState<string | null>(null);
  const [progress, setProgress] = useState<number | null>(null);
  const [installingWhisper, setInstallingWhisper] = useState(false);
  const [whisperProgress, setWhisperProgress] = useState<number | null>(null);

  const refresh = useCallback(async () => {
    // Every engine's install state plus the server's, one round trip fired
    // together rather than three awaits, so the page paints once.
    const [rows, catalogList, status] = await Promise.allSettled([
      invoke<BuiltinEngine[]>("audio_engines_builtin"),
      invoke<CatalogEngine[]>("audio_engines", { refresh: false }),
      invoke<AudioStatus>("audio_status"),
    ]);
    const nextBuiltins = rows.status === "fulfilled" ? rows.value : null;
    const nextCatalog = catalogList.status === "fulfilled" ? catalogList.value : null;
    const nextStatus = status.status === "fulfilled" ? status.value : null;
    if (nextBuiltins) setBuiltins(nextBuiltins);
    if (nextCatalog) setCatalog(nextCatalog);
    if (nextStatus) setStatus(nextStatus);
    // The snapshot is written only when everything answered: a partial read
    // would serve a page that claims one engine is gone because its command
    // raced a restart.
    if (nextBuiltins && nextCatalog && nextStatus) {
      snapshot = { builtins: nextBuiltins, catalog: nextCatalog, status: nextStatus };
    }
    if (rows.status === "rejected" && rows.reason != null)
      onError(`Could not read the engines: ${String(rows.reason)}`);
    // The catalog is remote metadata; failing there is a case rows-by-state
    // does not cover, so it is the one that reports.
    if (catalogList.status === "rejected")
      onError(`Could not read the release catalog: ${String(catalogList.reason)}`);
  }, [onError]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    let live = true;
    const stops: Array<() => void> = [];
    const track = (stop: () => void) => {
      if (live) stops.push(stop);
      else stop();
    };
    void listen<EngineProgress>("engine-download-progress", (event) => {
      const { id, downloaded, total } = event.payload;
      if (!id.startsWith("audiocpp-") && id !== "piper" && id !== "espeak") return;
      setProgress(downloaded / Math.max(total, 1));
      if (downloaded >= total && total > 0) setProgress(null);
    }).then(track);
    void listen<SttProgress>("stt-progress", (event) => {
      const { received, total } = event.payload;
      if (total) setWhisperProgress(received / total);
    }).then(track);
    void listen("downloads-installed", () => void refresh()).then(track);
    return () => {
      live = false;
      stops.forEach((stop) => stop());
    };
  }, [refresh]);

  const installCatalog = useCallback(
    (id: string) => {
      setInstalling(id);
      setProgress(0);
      void invoke("audio_engines_install", { id })
        .then(() => refresh())
        .catch((reason: unknown) =>
          onError(`The engine could not be installed: ${String(reason)}`),
        )
        .finally(() => {
          setInstalling(null);
          setProgress(null);
        });
    },
    [onError, refresh],
  );

  const installBuiltin = useCallback(
    (id: string) => {
      if (id === "whisper") {
        setInstallingWhisper(true);
        setWhisperProgress(0);
      } else {
        setInstalling(id);
        setProgress(0);
      }
      void invoke("audio_builtin_install", { id })
        .then(() => refresh())
        .catch((reason: unknown) =>
          onError(`The engine could not be installed: ${String(reason)}`),
        )
        .finally(() => {
          setInstallingWhisper(false);
          setInstalling(null);
          setProgress(null);
          setWhisperProgress(null);
        });
    },
    [onError, refresh],
  );

  const uninstall = useCallback(
    (id: string) => {
      void invoke("audio_builtin_uninstall", { id })
        .then(() => void refresh())
        .catch((reason: unknown) =>
          onError(`The engine could not be removed: ${String(reason)}`),
        );
    },
    [onError, refresh],
  );

  const start = useCallback(async () => {
    await invoke<AudioStatus>("audio_start").then(setStatus).catch((reason: unknown) =>
      onError(`The engine could not start: ${String(reason)}`),
    );
  }, [onError]);

  const stop = useCallback(async () => {
    await invoke<AudioStatus>("audio_stop").then(setStatus).catch((reason: unknown) =>
      onError(`The engine could not stop: ${String(reason)}`),
    );
  }, [onError]);

  return {
    builtins,
    catalog,
    status,
    installing,
    progress,
    installingWhisper,
    whisperProgress,
    refresh,
    installCatalog,
    installBuiltin,
    uninstall,
    start,
    stop,
  };
}
