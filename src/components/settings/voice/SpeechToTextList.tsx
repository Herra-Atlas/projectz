import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Check, Cpu, Download, LoaderCircle, Mic, Settings2, Trash2 } from "lucide-react";

import { formatBytes } from "../../../features/downloads/format";
import type { Notify } from "../../../features/notifications/types";
import type { SttModel, SttSettings } from "../../../features/stt/types";
import { useStt } from "../../../features/stt/useStt";
import { HoldToDelete } from "../../HoldToDelete";
import { ANCHOR_ATTRIBUTE } from "../../SettingsSection";

/**
 * The recognise half of the Speech to text page.
 *
 * Two things to install, and they are separate on purpose: the engine is a
 * whisper.cpp build with the Vulkan backend (16 MB packed, one per machine), and
 * a model is the file that actually transcribes (75 MB and up). The same engine
 * reads Whisper models and Parakeet ones -- they are different executables inside
 * one install, which is why the choice is a model, not an engine.
 *
 * Nothing downloads on its own: the engine is a button, the models are a page.
 */
export function SpeechToTextList({
  onDownload,
  onModelSettings,
  notify,
}: {
  onDownload: () => void;
  onModelSettings: (model: string) => void;
  notify: Notify;
}) {
  const { status, progress, installing, install } = useStt((message) => notify("error", message));
  const [models, setModels] = useState<SttModel[]>([]);
  const [chosen, setChosen] = useState<string | null>(null);

  const load = useCallback(() => {
    void invoke<SttModel[]>("stt_models").then(setModels).catch(() => undefined);
    void invoke<SttSettings>("stt_settings")
      .then((next) => setChosen(next.model))
      .catch(() => undefined);
  }, []);

  // Read on mount, and again whenever a download lands. The download page is a
  // sibling, so an install there has no other way to reach this list.
  useEffect(() => {
    load();
  }, [load]);
  useEffect(() => {
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
  }, [load]);

  const choose = useCallback(
    (model: SttModel) => {
      setChosen(model.path);
      void invoke<SttSettings>("stt_select_model", { model: model.path })
        .then((next) => setChosen(next.model))
        .catch((reason: unknown) => {
          setChosen((current) => (current === model.path ? null : current));
          notify("error", `That model could not be chosen: ${String(reason)}`);
        });
    },
    [notify],
  );

  const remove = useCallback(
    (model: SttModel) => {
      void invoke("stt_remove_model", { model: model.path })
        .then(load)
        .catch((reason: unknown) => notify("error", `The model could not be deleted: ${String(reason)}`));
    },
    [load, notify],
  );

  const engine = status?.engineInstalled === true;
  const percent = progress === null ? null : Math.round(progress * 100);
  // A model is chosen explicitly, or implied by being the only one installed,
  // which is what the backend does when nothing is stored.
  const active = chosen ?? models[0]?.path ?? null;

  return (
    <article>
      {/* Anchors exist on the rows themselves, so search lands where the setting is
          rather than on the page's top edge. */}
      <div className="flex min-h-[68px] items-center gap-2 px-4 py-3" {...{ [ANCHOR_ATTRIBUTE]: "stt.engine" }}>
        <span className="grid size-9 shrink-0 place-items-center rounded-md text-[var(--muted)]">
          <Cpu size={16} />
        </span>
        <div className="min-w-0 flex-1">
          <h3 className="truncate text-sm font-medium">Whisper &amp; Parakeet</h3>
          <p className="truncate text-xs text-[var(--quiet)]">
            {status === null
              ? "Checking…"
              : engine
                ? `${status.engineVersion} · ${models.length === 1 ? "1 model" : `${models.length} models`}`
                : "Engine not installed · Vulkan"}
          </p>
        </div>
        {!engine && (
          <button
            type="button"
            onClick={install}
            disabled={installing || status === null}
            className="inline-flex min-h-8 w-[104px] shrink-0 items-center justify-center gap-1.5 rounded-md border border-[var(--line)] px-2.5 text-[12px] font-medium text-[var(--text)] transition-colors hover:border-[var(--line-strong)] hover:bg-[var(--raised)] disabled:opacity-40 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
          >
            {installing ? <LoaderCircle size={13} className="animate-spin" /> : <Download size={13} />}
            <span>{installing ? (percent === null ? "Starting" : `${percent}%`) : "Install"}</span>
          </button>
        )}
        {engine && (
          <button
            type="button"
            onClick={onDownload}
            className="inline-flex min-h-8 shrink-0 items-center gap-1.5 rounded-md border border-[var(--line)] px-2.5 text-[12px] text-[var(--text)] transition-colors hover:border-[var(--line-strong)] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
          >
            <Download size={13} />
            <span>Download model</span>
          </button>
        )}
      </div>

      <div className="mb-3 ml-[2.75rem] border-l border-[var(--line)] py-1 pl-3" {...{ [ANCHOR_ATTRIBUTE]: "stt.models" }}>
        {models.map((model) => (
          <div key={model.path} className="flex items-center gap-1">
            <button
              type="button"
              onClick={() => choose(model)}
              title={model.path}
              aria-pressed={model.path === active}
              className="flex min-h-9 min-w-0 flex-1 items-center gap-3 rounded-md py-1.5 pl-1 text-left hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)]"
            >
              <Mic
                size={14}
                className={`shrink-0 ${model.path === active ? "text-[var(--accent)]" : "text-[var(--quiet)]"}`}
              />
              <div className="min-w-0 flex-1">
                <h4 className="truncate text-xs font-medium text-[var(--muted)]">{model.name}</h4>
                <p className="truncate text-[11px] text-[var(--quiet)]">
                  {model.audio
                    ? `audio.cpp · ${model.family !== "" ? model.family.replace(/_/g, " ") : "unknown"}`
                    : model.parakeet ? "Parakeet" : "Whisper"}{" "}
                  · {formatBytes(model.size)}
                </p>
              </div>
              {model.path === active && <Check size={14} className="shrink-0 text-[var(--accent)]" />}
            </button>
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
            {engine
              ? "No models yet. Download one — Whisper or Parakeet — to start transcribing."
              : "Install the engine, then download a model."}
          </p>
        )}
      </div>
    </article>
  );
}
