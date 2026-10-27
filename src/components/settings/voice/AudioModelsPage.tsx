import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Download, Settings2, Trash2, Volume2, Mic } from "lucide-react";

import { formatBytes } from "../../../features/downloads/format";
import type { AudioModelRow } from "../../../features/audio/useAudioEngine";
import type { Notify } from "../../../features/notifications/types";
import { HoldToDelete } from "../../HoldToDelete";
import { FlashHighlight, SettingsPageTitle } from "../../SettingsSection";

/**
 * The audio models page, shaped like the Text to speech page.
 *
 * One list of everything audio.cpp serves -- speech and recognition both, badged
 * per row -- with the download page a button away and each model's settings
 * behind its gear. The page reads the folder, so what it shows is what will be
 * served after the next engine restart.
 */
export function AudioModelsPage({
  onDownload,
  onModelSettings,
  notify,
}: {
  onDownload: () => void;
  onModelSettings: (path: string) => void;
  notify: Notify;
}) {
  const [models, setModels] = useState<AudioModelRow[]>([]);

  const load = () => {
    void invoke<AudioModelRow[]>("audio_models").then(setModels).catch(() => undefined);
  };

  // Read on mount and again whenever a download lands: the download page is a
  // sibling, so an install there reaches this list only through the event.
  useEffect(() => {
    load();
    let live = true;
    let stop: (() => void) | undefined;
    void listen("downloads-installed", () => load()).then((unlisten) => {
      if (live) stop = unlisten;
      else unlisten();
    });
    return () => {
      live = false;
      stop?.();
    };
  }, []);

  const remove = (model: AudioModelRow) => {
    void invoke("stt_remove_model", { model: model.path })
      .then(load)
      .catch((reason: unknown) =>
        notify("error", `The model could not be deleted: ${String(reason)}`),
      );
  };

  return (
    <>
      <SettingsPageTitle>
        <h2 className="text-[17px] font-semibold tracking-tight">
          <FlashHighlight anchor="audio.models" />
          Audio models
        </h2>
      </SettingsPageTitle>
      <article>
        <div className="flex min-h-[68px] items-center gap-2 px-4 py-3">
          <span className="grid size-9 shrink-0 place-items-center rounded-md text-[var(--muted)]">
            <Download size={16} />
          </span>
          <div className="min-w-0 flex-1">
            <h3 className="truncate text-sm font-medium">audio.cpp models</h3>
            <p className="truncate text-xs text-[var(--quiet)]">
              {models === null
                ? "Checking…"
                : models.length === 0
                  ? "No models installed"
                  : models.length === 1
                    ? "1 model"
                    : `${models.length} models`}
            </p>
          </div>
          <button
            type="button"
            onClick={onDownload}
            className="inline-flex min-h-8 shrink-0 items-center gap-1.5 rounded-md border border-[var(--line)] px-2.5 text-[12px] text-[var(--text)] transition-colors hover:border-[var(--line-strong)] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
          >
            <Download size={13} />
            <span>Download model</span>
          </button>
        </div>

        <div className="mb-3 ml-[2.75rem] border-l border-[var(--line)] py-1 pl-3">
          {models.map((model) => (
            <div key={model.path} className="flex items-center gap-1">
              <div className="flex min-h-9 min-w-0 flex-1 items-center gap-3 rounded-md py-1.5 pl-1 text-left" title={model.path}>
                {model.task === "asr" ? (
                  <Mic size={14} className="shrink-0 text-[var(--quiet)]" />
                ) : (
                  <Volume2 size={14} className="shrink-0 text-[var(--quiet)]" />
                )}
                <div className="min-w-0 flex-1">
                  <h4 className="truncate text-xs font-medium text-[var(--muted)]">{model.name}</h4>
                  <p className="truncate text-[11px] text-[var(--quiet)]">
                    {model.task === "tts" ? "speech" : model.task === "asr" ? "recognition" : model.task === "wake" ? "wake word" : "unknown family"} ·{" "}
                    {model.family.replace(/_/g, " ")} · {formatBytes(model.size)}
                  </p>
                </div>
              </div>
              <button
                type="button"
                onClick={() => onModelSettings(model.path)}
                aria-label={`${model.name} settings`}
                title="Model settings"
                className="grid size-8 shrink-0 place-items-center rounded-md text-[var(--quiet)] hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
              >
                <Settings2 size={14} />
              </button>
              <HoldToDelete
                onConfirm={() => remove(model)}
                label={`Delete ${model.name}`}
                title="Delete model"
                className="grid size-8 shrink-0 place-items-center rounded-md text-[var(--quiet)] hover:bg-[var(--raised)] hover:text-[var(--danger)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
              >
                <Trash2 size={14} />
              </HoldToDelete>
            </div>
          ))}
          {models.length === 0 && (
            <p className="py-2 text-xs text-[var(--quiet)]">
              No models yet. Download one — Chatterbox, Kokoro, Qwen3, Parakeet and more — to start.
            </p>
          )}
        </div>

        {/* Phonemizing families read through eSpeak-ng, which the engine's
            Windows release does not bundle -- the engine answers its own
            "install the shared library" error when one of these is used and
            the machine has none. The hint points at the app's own installer,
            which keeps the component inside the app's data folder; families
            that do not phonemize never need it. */}
        {models.some((model) =>
          ["inflect", "sano", "piper", "kitten"].some((token) =>
            model.family.startsWith(token),
          ),
        ) && (
          <p className="border-l border-[var(--line)] px-4 pb-3 text-[11px] text-[var(--quiet)]">
            Inflect, SanoTTS, Piper and Kitten voices phonemize through{" "}
            <span className="text-[var(--muted)]">eSpeak-ng</span> — install it from the{" "}
            <span className="text-[var(--muted)]">Audio engine page</span> if the engine's logs
            say it is missing; everything else runs without it.
          </p>
        )}
      </article>
    </>
  );
}
