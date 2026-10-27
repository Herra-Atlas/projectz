import { useState } from "react";
import { Disc3, Trash2 } from "lucide-react";

import type { BuiltinEngine, CatalogEngine } from "../../../features/audio/useAudioEngine";
import { useAudioEngine } from "../../../features/audio/useAudioEngine";
import type { Notify } from "../../../features/notifications/types";
import { HoldToDelete } from "../../HoldToDelete";
import { ANCHOR_ATTRIBUTE } from "../../SettingsSection";
import HelpMark from "../../HelpMark";

/**
 * What each engine is for, in one line.
 *
 * Kept here rather than on the backend rows because it is copy, not state: the
 * engine list changes with what is installed, and a hint that travelled through
 * the catalog would be one more thing to keep in step with nothing.
 */
const HINTS: Record<string, string> = {
  audiocpp:
    "One server for voices, speech recognition, wake words and voice activity. Voices and models install from the audio.cpp page.",
  whisper:
    "Transcribes with Whisper or Parakeet models, on Vulkan. Runs locally and only while a recording is being turned into text.",
  piper:
    "A small, fast voice set for reading replies aloud. CPU only — no GPU memory, no server.",
  espeak:
    "A phonemizer, not an engine: a few audio.cpp voices need it to turn text into sounds. It never speaks on its own.",
  sherpa:
    "Speaker embeddings for Voice recognition: turns a voice into numbers so a wake word can tell who spoke. Does not transcribe or speak.",
  openwakeword:
    "Runs wake-word models trained for openWakeWord, including the community collection. Installs ONNX Runtime plus the two models every word shares; each word is a small extra file.",
};

/**
 * The audio engine page: the executables, and nothing else.
 *
 * Whisper, Piper, and audio.cpp are the engines, each with its install and its
 * uninstall; eSpeak-ng is not an engine, it is audio.cpp's phonemizer add-on,
 * so its row sits under the engines at the bottom of the page. audio.cpp
 * installs from a live release catalog the way the llama.cpp engines do; the
 * others are pinned builds whose checksums live in the source.
 *
 * The audio.cpp row carries the server's own switch: one row per engine means
 * the on/off is where the engine's name is, not a second row that repeats it.
 */
export function AudioEnginePage({ notify }: { notify: Notify }) {
  const engine = useAudioEngine((message) => notify("error", message));
  const [releasesOpen, setReleasesOpen] = useState(false);
  const builtin = (id: string) => engine.builtins.find((row) => row.id === id);

  const audiocpp = builtin("audiocpp");
  const installed = audiocpp?.installed === true;

  return (
    <article>
      {builtin("whisper") && <EngineRow engine={builtin("whisper")!} hook={engine} />}
      {builtin("piper") && <EngineRow engine={builtin("piper")!} hook={engine} />}

      {/* audio.cpp: catalog-installed, server switched in the same row. */}
      <div className="flex min-h-[68px] items-center gap-2 px-4 py-3" {...{ [ANCHOR_ATTRIBUTE]: "audio.engine" }}>
        <span className="grid size-9 shrink-0 place-items-center rounded-md text-[var(--muted)]">
          <Disc3 size={16} />
        </span>
        <div className="min-w-0 flex-1">
          <h3 className="flex items-center gap-1.5 truncate text-sm font-medium">
            audio.cpp
            {HINTS.audiocpp && <HelpMark hint={HINTS.audiocpp} />}
          </h3>
          <p className="truncate text-xs text-[var(--quiet)]">
            {audiocpp === undefined ? (
              <span className="inline-block h-3 w-40 animate-pulse rounded bg-[var(--raised)]" />
            ) : installed ? (
              `${audiocpp.version} · ${audiocpp.assets === 1 ? "1 model" : `${audiocpp.assets} models`}`
            ) : (
              "Engine not installed · Vulkan"
            )}
          </p>
        </div>
        {/* No server switch: the engine starts when something needs it and
            stops when that thing is switched off, so a switch here was a second
            answer to a question the features already answer. */}
        {!installed && (
          <button
            type="button"
            onClick={() => setReleasesOpen((open) => !open)}
            disabled={engine.installing !== null}
            className="inline-flex min-h-8 shrink-0 items-center gap-1.5 rounded-md border border-[var(--line)] px-2.5 text-[12px] text-[var(--text)] transition-colors hover:border-[var(--line-strong)] hover:bg-[var(--raised)] disabled:opacity-40 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
          >
            <span>{releasesOpen ? "Hide releases" : "Choose release"}</span>
          </button>
        )}
        {installed && (
          <HoldToDelete
            onConfirm={() => engine.uninstall("audiocpp")}
            label="Uninstall audio.cpp"
            title="Uninstall engine"
            className="grid size-8 shrink-0 place-items-center rounded-md text-[var(--quiet)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--danger)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
          >
            <Trash2 size={14} />
          </HoldToDelete>
        )}
      </div>

      {/* Releases come out of the install row's own border, the way a settings
          drill-down reads -- until one is installed, at which point the row
          names the version and the list goes away. */}
      {!installed && releasesOpen && (
        <div className="mb-3 ml-[2.75rem] border-l border-[var(--line)] py-1 pl-3">
          {engine.catalog.length === 0 && (
            <p className="py-2 text-xs text-[var(--quiet)]">
              No releases were listed. Check the connection and try again.
            </p>
          )}
          {engine.catalog.map((row) => (
            <ReleaseRow
              key={row.id}
              row={row}
              installing={engine.installing === row.id}
              percent={engine.progress === null ? null : Math.round(engine.progress * 100)}
              onInstall={engine.installCatalog}
            />
          ))}
        </div>
      )}

      {/* eSpeak is an add-on, not an engine: audio.cpp's own phonemizer, used
          only by a handful of families. Bottom of the page, below the engines,
          and its row is the plain add-on read rather than an engine row that
          reports "0 models". */}
      <div className="mt-1 border-t border-[var(--line)]" />
      {builtin("espeak") && <EngineRow engine={builtin("espeak")!} hook={engine} />}
      {builtin("sherpa") && <EngineRow engine={builtin("sherpa")!} hook={engine} />}
      {builtin("openwakeword") && <EngineRow engine={builtin("openwakeword")!} hook={engine} />}
    </article>
  );
}

/** One catalog release: version, size, one download button. */
function ReleaseRow({
  row,
  installing,
  percent,
  onInstall,
}: {
  row: CatalogEngine;
  installing: boolean;
  percent: number | null;
  onInstall: (id: string) => void;
}) {
  return (
    <div className="flex min-h-10 items-center gap-3">
      <div className="min-w-0 flex-1">
        <h4 className="truncate text-xs font-medium text-[var(--muted)]">{row.version}</h4>
        <p className="truncate text-[11px] text-[var(--quiet)]">
          {row.assetName} · {(row.sizeBytes / (1024 * 1024)).toFixed(0)} MB
        </p>
      </div>
      <button
        type="button"
        onClick={() => onInstall(row.id)}
        disabled={installing}
        className="inline-flex min-h-8 w-[104px] shrink-0 items-center justify-center gap-1.5 rounded-md border border-[var(--line)] px-2.5 text-[12px] font-medium text-[var(--text)] transition-colors hover:border-[var(--line-strong)] hover:bg-[var(--raised)] disabled:opacity-40 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
      >
        <span>{installing ? (percent === null ? "Starting" : `${percent}%`) : "Download"}</span>
      </button>
    </div>
  );
}

/** One pinned engine: whisper or Piper, install and (held) uninstall. */
function EngineRow({
  engine,
  hook,
}: {
  engine: BuiltinEngine;
  hook: ReturnType<typeof useAudioEngine>;
}) {
  // Whisper runs on its own older event; every other pinned engine shares the
  // catalog's. Keying the button on the *id* is what stops a second press
  // landing on a row that is already downloading.
  const installing =
    engine.id === "whisper" ? hook.installingWhisper : hook.installing === engine.id;
  const percent =
    engine.id === "whisper"
      ? hook.whisperProgress === null
        ? null
        : Math.round(hook.whisperProgress * 100)
      : engine.id === "piper" || engine.id === "espeak" || engine.id === "sherpa"
        ? hook.progress === null
          ? null
          : Math.round(hook.progress * 100)
        : null;
  const noun = engine.id === "piper" ? "voice" : "model";
  // The eSpeak row is a component, not an engine: installed it names what it
  // serves, never an asset count that would read "0 models" for a shared
  // library that runs nothing on its own.
  const subtitle =
    engine.id === "espeak"
      ? engine.installed
        ? `${engine.version} · reads through it: Inflect, SanoTTS, Kitten voices`
        : "Optional — Inflect, SanoTTS, Kitten voices need it"
      : engine.installed
        ? `${engine.version} · ${
            engine.assets === 1 ? `1 ${noun}` : `${engine.assets} ${noun}s`
          }`
        : engine.id === "piper"
          ? "Engine not installed"
          // The only engine that is optional by feature rather than by need:
          // nothing else uses it, so its absence costs nothing until the
          // Voice recognition page is asked for something.
          : engine.id === "sherpa"
            ? "Optional — only needed for voice recognition"
            : "Engine not installed · Vulkan";
  return (
    <div className="flex min-h-[68px] items-center gap-2 px-4 py-3">
      <span className="grid size-9 shrink-0 place-items-center rounded-md text-[var(--muted)]">
        <Disc3 size={16} />
      </span>
      <div className="min-w-0 flex-1">
        <h3 className="flex items-center gap-1.5 truncate text-sm font-medium">
          {engine.name}
          {HINTS[engine.id] && <HelpMark hint={HINTS[engine.id]} />}
        </h3>
        <p className="truncate text-xs text-[var(--quiet)]">{subtitle}</p>
      </div>
      {!engine.installed ? (
        <button
          type="button"
          onClick={() => hook.installBuiltin(engine.id)}
          disabled={installing}
          className="inline-flex min-h-8 w-[104px] shrink-0 items-center justify-center gap-1.5 rounded-md border border-[var(--line)] px-2.5 text-[12px] font-medium text-[var(--text)] transition-colors hover:border-[var(--line-strong)] hover:bg-[var(--raised)] disabled:opacity-40 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
        >
          <span>{installing ? (percent === null ? "Starting" : `${percent}%`) : "Install"}</span>
        </button>
      ) : (
        <HoldToDelete
          onConfirm={() => hook.uninstall(engine.id)}
          label={`Uninstall ${engine.name}`}
          title="Uninstall engine"
          className="grid size-8 shrink-0 place-items-center rounded-md text-[var(--quiet)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--danger)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
        >
          <Trash2 size={14} />
        </HoldToDelete>
      )}
    </div>
  );
}
