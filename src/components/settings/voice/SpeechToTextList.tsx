import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  AudioLines,
  Check,
  Cpu,
  Download,
  LoaderCircle,
  Mic,
  Settings2,
  Trash2,
} from "lucide-react";

import { formatBytes } from "../../../features/downloads/format";
import type { Notify } from "../../../features/notifications/types";
import type { SttModel, SttSettings } from "../../../features/stt/types";
import { useStt } from "../../../features/stt/useStt";
import { HoldToDelete } from "../../HoldToDelete";
import Select from "../../Select";
import { ANCHOR_ATTRIBUTE, SettingRow, SettingsSection, Toggle } from "../../SettingsSection";

/**
 * The recognise half of the Speech to text page.
 *
 * Two engines can transcribe, and they keep their files in two folders:
 * whisper.cpp reads `ggml-*.bin` (Parakeet through its own executable in the
 * same install), audio.cpp serves its own gguf packages from `models/audio`.
 * The choice is a model, not an engine, so both are listed here and grouped by
 * the folder they came from.
 *
 * Nothing downloads on its own: the engine is a button, the models are a page.
 */
export function SpeechToTextList({
  onDownload,
  onAudioDownload,
  onModelSettings,
  notify,
}: {
  onDownload: () => void;
  onAudioDownload: () => void;
  onModelSettings: (model: string) => void;
  notify: Notify;
}) {
  const { status, progress, installing, install } = useStt((message) => notify("error", message));
  const [models, setModels] = useState<SttModel[]>([]);
  const [chosen, setChosen] = useState<string | null>(null);
  // Kept beside `chosen` because both live in the same record, and the list is
  // the only place that reads it.
  const [keepLoaded, setKeepLoaded] = useState(false);
  const [autoSend, setAutoSend] = useState(false);
  const [silenceMs, setSilenceMs] = useState(1500);

  const load = useCallback(() => {
    void invoke<SttModel[]>("stt_models").then(setModels).catch(() => undefined);
    void invoke<SttSettings>("stt_settings")
      .then((next) => {
        setChosen(next.model);
        setKeepLoaded(next.keepLoaded);
        setAutoSend(next.autoSend);
        setSilenceMs(next.silenceMs);
      })
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
        .catch((reason: unknown) =>
          notify("error", `The model could not be deleted: ${String(reason)}`),
        );
    },
    [load, notify],
  );

  const engine = status?.engineInstalled === true;
  const percent = progress === null ? null : Math.round(progress * 100);
  // A model is chosen explicitly, or implied by being the only one installed,
  // which is what the backend does when nothing is stored.
  const active = chosen ?? models[0]?.path ?? null;
  // Grouped by the engine that serves the file, which is the one fact that
  // decides what installing more of it even means: whisper needs a pinned build,
  // audio.cpp needs its server, and a speech GGUF needs neither.
  const whisper = models.filter((model) => model.engine === "whisper");
  const audio = models.filter((model) => model.engine === "audiocpp");
  const speech = models.filter((model) => model.engine === "llamacpp");

  return (
    <>
      <article>
        {/* Anchors exist on the rows themselves, so search lands where the
            setting is rather than on the page's top edge. */}
        <div
          className="flex min-h-[68px] items-center gap-2 px-4 py-3"
          {...{ [ANCHOR_ATTRIBUTE]: "stt.engine" }}
        >
          <span className="grid size-9 shrink-0 place-items-center rounded-md text-[var(--muted)]">
            <Cpu size={16} />
          </span>
          <div className="min-w-0 flex-1">
            <h3 className="truncate text-sm font-medium">Whisper &amp; Parakeet</h3>
            <p className="truncate text-xs text-[var(--quiet)]">
              {status === null
                ? "Checking…"
                : engine
                  ? `${status.engineVersion} · ${whisper.length === 1 ? "1 model" : `${whisper.length} models`}`
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

        <div
          className="mb-3 ml-[2.75rem] border-l border-[var(--line)] py-1 pl-3"
          {...{ [ANCHOR_ATTRIBUTE]: "stt.models" }}
        >
          {whisper.map((model) => (
            <ModelRow
              key={model.path}
              model={model}
              active={model.path === active}
              onChoose={choose}
              onSettings={onModelSettings}
              onRemove={remove}
            />
          ))}

          {whisper.length === 0 && (
            <p className="py-2 text-xs text-[var(--quiet)]">
              {engine
                ? "No models yet. Download one — Whisper or Parakeet — to start transcribing."
                : "Install the engine, then download a model."}
            </p>
          )}
        </div>
      </article>

      <article>
        <div
          className="flex min-h-[68px] items-center gap-2 px-4 py-3"
          {...{ [ANCHOR_ATTRIBUTE]: "stt.audio" }}
          >
            <span className="grid size-9 shrink-0 place-items-center rounded-md text-[var(--muted)]">
              <AudioLines size={16} />
            </span>
            <div className="min-w-0 flex-1">
              <h3 className="truncate text-sm font-medium">audio.cpp</h3>
              <p className="truncate text-xs text-[var(--quiet)]">
                {audio.length === 1 ? "1 model" : `${audio.length} models`}
              </p>
            </div>
            <button
              type="button"
              onClick={onAudioDownload}
              className="inline-flex min-h-8 shrink-0 items-center gap-1.5 rounded-md border border-[var(--line)] px-2.5 text-[12px] text-[var(--text)] transition-colors hover:border-[var(--line-strong)] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
            >
              <Download size={13} />
              <span>Download model</span>
            </button>
          </div>
          <div
            className="mb-3 ml-[2.75rem] border-l border-[var(--line)] py-1 pl-3"
            {...{ [ANCHOR_ATTRIBUTE]: "stt.audioModels" }}
          >
            {/* One delete rule for both folders: `stt_remove_model` deletes an
                audio.cpp file under `models/audio` and a whisper one under
                `models/stt`, so a model can be removed from whichever list it
                was found in. */}
            {audio.map((model) => (
              <ModelRow
                key={model.path}
                model={model}
                active={model.path === active}
                onChoose={choose}
                onSettings={onModelSettings}
                onRemove={remove}
              />
            ))}
            {audio.length === 0 && (
              <p className="py-2 text-xs text-[var(--quiet)]">
                No audio.cpp model yet. Download one, or add it on the Audio models page.
              </p>
            )}
          </div>
      </article>

      <article>
        <div
          className="flex min-h-[68px] items-center gap-2 px-4 py-3"
          {...{ [ANCHOR_ATTRIBUTE]: "stt.speech" }}
        >
          <span className="grid size-9 shrink-0 place-items-center rounded-md text-[var(--muted)]">
            <AudioLines size={16} />
          </span>
          <div className="min-w-0 flex-1">
            <h3 className="truncate text-sm font-medium">llama.cpp speech models</h3>
            <p className="truncate text-xs text-[var(--quiet)]">
              {speech.length === 1 ? "1 model" : `${speech.length} models`}
            </p>
          </div>
        </div>
        <div
          className="mb-3 ml-[2.75rem] border-l border-[var(--line)] py-1 pl-3"
          {...{ [ANCHOR_ATTRIBUTE]: "stt.speechModels" }}
        >
          {/* No delete and no download here: the file is registered in the model
              library, where it is installed and removed with its engine. */}
          {speech.map((model) => (
            <ModelRow
              key={model.path}
              model={model}
              active={model.path === active}
              onChoose={choose}
              onSettings={onModelSettings}
            />
          ))}
          {speech.length === 0 && (
            <p className="py-2 text-xs text-[var(--quiet)]">
              A GGUF model with an audio projector is listed here and is used instead of the
              engines above. Add one in Settings · Local.
            </p>
          )}
        </div>
      </article>

      <SettingsSection title="Dictation">
        <SettingRow
          anchor="stt.autoSend"
          label="Send when I stop talking"
          description="A finished transcript goes straight out as your message, instead of waiting in the composer for you to press send."
          hint="Off, the words land in the draft and you can fix them first — which is what you want for anything you would not send unedited."
          control={
            <Toggle
              checked={autoSend}
              onChange={(next) => {
                setAutoSend(next);
                void invoke<SttSettings>("stt_set_auto_send", { send: next })
                  .then((saved) => setAutoSend(saved.autoSend))
                  .catch((reason: unknown) => {
                    setAutoSend(!next);
                    notify("error", `That could not be saved: ${String(reason)}`);
                  });
              }}
              label="Send when I stop talking"
            />
          }
        />
        <SettingRow
          anchor="stt.silence"
          label="Wait for silence"
          description="How long a pause ends the recording once you have started talking. A short one feels quick and cuts off thinkers; a long one waits through a mid-sentence pause."
          control={
            <Select
              label="Wait for silence"
              value={String(silenceMs)}
              onChange={(value) => {
                const next = Number(value);
                setSilenceMs(next);
                void invoke<SttSettings>("stt_set_silence", { silenceMs: next })
                  .then((saved) => setSilenceMs(saved.silenceMs))
                  .catch((reason: unknown) => {
                    notify("error", `That could not be saved: ${String(reason)}`);
                    load();
                  });
              }}
              options={[
                { value: "800", label: "0.8 s (default)" },
                { value: "1200", label: "1.2 s" },
                { value: "1500", label: "1.5 s" },
                { value: "2000", label: "2 s" },
                { value: "3000", label: "3 s" },
                { value: "4000", label: "4 s" },
              ]}
            />
          }
        />
      </SettingsSection>

      <SettingsSection title="Memory">
        <SettingRow
          anchor="stt.keepLoaded"
          label="Keep the model loaded"
          description="Reads the model once and holds it between recordings, so a press starts transcribing instead of loading first. Costs memory for as long as the app is open."
          hint="Off, every recording runs the engine's command-line tool, which reads the model from disk, transcribes, and exits. On, a small local server holds the weights instead; switching models or turning this off stops it. Parakeet has no server, so it always loads per recording, and audio.cpp models are held by their own server either way."
          control={
            <Toggle
              checked={keepLoaded}
              onChange={(next) => {
                setKeepLoaded(next);
                void invoke<SttSettings>("stt_set_keep_loaded", { keep: next })
                  .then((saved) => setKeepLoaded(saved.keepLoaded))
                  .catch((reason: unknown) => {
                    // The record is the truth: a refused save must not leave the
                    // switch claiming a state the backend does not hold.
                    setKeepLoaded(!next);
                    notify("error", `That could not be saved: ${String(reason)}`);
                  });
              }}
              label="Keep the model loaded"
            />
          }
        />
      </SettingsSection>
    </>
  );
}

/** One model row: choose, open its settings, and -- where allowed -- delete. */
function ModelRow({
  model,
  active,
  onChoose,
  onSettings,
  onRemove,
}: {
  model: SttModel;
  active: boolean;
  onChoose: (model: SttModel) => void;
  onSettings: (model: string) => void;
  onRemove?: (model: SttModel) => void;
}) {
  return (
    <div className="flex items-center gap-1">
      <button
        type="button"
        onClick={() => onChoose(model)}
        title={model.path}
        aria-pressed={active}
        className="flex min-h-9 min-w-0 flex-1 items-center gap-3 rounded-md py-1.5 pl-1 text-left hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)]"
      >
        <Mic size={14} className={`shrink-0 ${active ? "text-[var(--accent)]" : "text-[var(--quiet)]"}`} />
        <div className="min-w-0 flex-1">
          <h4 className="truncate text-xs font-medium text-[var(--muted)]">{model.name}</h4>
          <p className="truncate text-[11px] text-[var(--quiet)]">
            {model.engine === "audiocpp"
              ? `audio.cpp · ${model.family !== "" ? model.family.replace(/_/g, " ") : "unknown"}`
              : model.engine === "llamacpp"
                ? "llama.cpp · speech model"
                : model.parakeet
                  ? "Parakeet"
                  : "Whisper"}{" "}
            · {formatBytes(model.size)}
          </p>
        </div>
        {active && <Check size={14} className="shrink-0 text-[var(--accent)]" />}
      </button>
      <button
        type="button"
        onClick={() => onSettings(model.path)}
        aria-label={`${model.name} settings`}
        title="Model settings"
        className="grid size-8 shrink-0 place-items-center rounded-md text-[var(--quiet)] hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
      >
        <Settings2 size={14} />
      </button>
      {onRemove && (
        <HoldToDelete
          onConfirm={() => onRemove(model)}
          label={`Delete ${model.name}`}
          title="Delete model"
          className="grid size-8 shrink-0 place-items-center rounded-md text-[var(--quiet)] hover:bg-[var(--raised)] hover:text-[var(--danger)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
        >
          <Trash2 size={14} />
        </HoldToDelete>
      )}
    </div>
  );
}
