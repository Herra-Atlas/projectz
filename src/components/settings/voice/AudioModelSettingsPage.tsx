import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { AudioLines, ArrowLeft, Play, RotateCcw, X } from "lucide-react";

import type { AudioOption } from "../../../features/audio/useAudioModel";
import { useAudioModel } from "../../../features/audio/useAudioModel";
import { useVoiceTest } from "../../../features/tts/useVoiceTest";
import type { Notify } from "../../../features/notifications/types";
import { SettingsPageTitle } from "../../SettingsSection";

/**
 * One audio.cpp model's settings, row by row, from its own spec.
 *
 * The spec (`model_specs/<family>.json` inside the engine install) is the
 * source of what this page may ask: a text option is a text field, an enum is
 * a dropdown, a load option says it needs an engine restart. What the user
 * sets is saved per model as they type -- the request the engine receives is
 * the record the page shows.
 *
 * `required` options are marked rather than enforced here: the engine
 * validates the request, and doubling a validator that would drift is how
 * silent refusals start.
 */
export function AudioModelSettingsPage({
  modelPath,
  onBack,
  backLabel = "Back to audio models",
  notify,
}: {
  modelPath: string;
  onBack: () => void;
  /** Where the bottom control says it returns to; a wake pack came from another page. */
  backLabel?: string;
  notify: Notify;
}) {
  const { spec, values, set, reset } = useAudioModel(
    modelPath,
    onBack,
    (message) => notify("error", message),
  );
  // The corner button is the fastest read on whether any of this works: the
  // model speaks one short line with its own stored options in force.
  const { testing, test } = useVoiceTest((message) =>
    notify("error", `Could not speak: ${message}`),
  );
  // A speech model is also a voice, so its pace rides the same per-voice
  // speed the Piper page stores -- one record, whichever page reaches it.
  // `tts_speed` answers a view (speed with its allowed bounds), not a bare
  // number, which is exactly what the first attempt assumed and crashed on.
  const isSpeech = spec?.tasks.includes("tts") === true;
  const [pace, setPace] = useState<{ speed: number; minSpeed: number; maxSpeed: number } | null>(
    null,
  );
  useEffect(() => {
    if (!isSpeech) return;
    let live = true;
    void invoke<{ speed: number; minSpeed: number; maxSpeed: number }>("tts_speed", {
      voice: modelPath,
    })
      .then((view) => {
        if (live) setPace(view);
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, [isSpeech, modelPath]);

  const saveSpeed = async (next: number) => {
    const previous = pace;
    if (pace) setPace({ ...pace, speed: next });
    try {
      await invoke("tts_set_speed", { voice: modelPath, speed: next });
    } catch (reason) {
      setPace(previous);
      notify("error", `That could not be saved: ${String(reason)}`);
    }
  };

  if (spec === null) return null;
  const request = spec.options.filter((option) => option.section === "request");
  const load = spec.options.filter((option) => option.section === "load");
  const session = spec.options.filter((option) => option.section === "session");
  const canClone = Object.values(spec.capabilities)
    .flat()
    .includes("speaker_reference");
  const traits = Object.entries(spec.capabilities)
    .flatMap(([task, tags]) => (task === spec.task || task === "tts" || task === "asr" ? tags : []))
    .filter((tag, index, all) => all.indexOf(tag) === index);

  return (
    <>
      <SettingsPageTitle>
        <h2 className="text-[17px] font-semibold tracking-tight">
          {spec.displayName ?? spec.name}
        </h2>
        {/* A test speaks one line through the model, so it belongs to a voice.
            An ASR or wake family is opened here for its own options, and a
            button that synthesised with them could only fail. */}
        {isSpeech && (
          <button
            type="button"
            onClick={() => void test(modelPath)}
            disabled={testing}
            className="inline-flex min-h-8 shrink-0 items-center gap-1.5 rounded-md border border-[var(--line)] px-2.5 text-[12px] text-[var(--text)] transition-colors hover:border-[var(--line-strong)] hover:bg-[var(--raised)] disabled:opacity-40 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
          >
            <Play size={13} />
            <span>{testing ? "Speaking…" : "Test voice"}</span>
          </button>
        )}
      </SettingsPageTitle>
      <article>
        {/* What the family is, before any option row: task tags first, then the
            family id -- the two facts a reader checks against the docs. */}
        <div className="flex min-h-[56px] items-center gap-2 px-4 py-3">
          <div className="min-w-0 flex-1">
            <p className="truncate text-xs text-[var(--quiet)]">
              {spec.tasks.length > 0 ? spec.tasks.map((task) => friendly(task)).join(" · ") : spec.task} ·{" "}
              {spec.family.replace(/_/g, " ")}
            </p>
            {traits.length > 0 && (
              <div className="mt-1 flex flex-wrap gap-1.5">
                {traits.map((tag) => (
                  <span
                    key={tag}
                    title={`${spec.displayName ?? spec.family} can ${flexibleCap(tag)} — its spec reports this.`}
                    className="rounded border border-[var(--line)] px-1.5 py-0.5 text-[11px] text-[var(--quiet)]"
                  >
                    {flexibleCap(tag)}
                  </span>
                ))}
              </div>
            )}
          </div>
          {spec.options.length > 0 && (
            <button
              type="button"
              onClick={() => void reset()}
              className="inline-flex min-h-8 shrink-0 items-center gap-1.5 rounded-md border border-[var(--line)] px-2.5 text-[12px] text-[var(--text)] transition-colors hover:border-[var(--line-strong)] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
            >
              <RotateCcw size={13} />
              <span>Reset to defaults</span>
            </button>
          )}
        </div>

        {canClone && (
          <CloneVoiceRow
            values={values}
            onChange={(name, next) => void set(name, next)}
            notify={notify}
          />
        )}

        {request.length === 0 && load.length === 0 && session.length === 0 && (
          <div className="mb-3 ml-[2.75rem] border-l border-[var(--line)] py-2 pl-3 text-xs text-[var(--quiet)]">
            This family ships no options. It takes text and its rate, and its defaults are its own.
          </div>
        )}
        {isSpeech && pace !== null && (
          <div className="ml-[2.75rem] border-l border-[var(--line)] py-1 pl-3">
            <div className="flex min-h-[52px] items-center gap-3 py-1">
              <div className="min-w-0 flex-1">
                <h4 className="truncate text-xs font-medium text-[var(--muted)]">Speech speed</h4>
                <p className="truncate text-[11px] text-[var(--quiet)]">
                  How fast this voice reads. Stored per voice, like every engine's pace.
                </p>
              </div>
              <input
                type="range"
                min={pace.minSpeed}
                max={pace.maxSpeed}
                step={0.05}
                value={pace.speed}
                onChange={(event) => void saveSpeed(Number(event.target.value))}
                className="w-[160px] accent-[var(--accent)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
                aria-label="Speech speed"
              />
              <span className="w-10 shrink-0 text-right text-[12px] tabular-nums text-[var(--muted)]">
                {pace.speed.toFixed(2)}×
              </span>
            </div>
          </div>
        )}
        {request.length > 0 && (
          <div className="mb-3 ml-[2.75rem] border-l border-[var(--line)] py-1 pl-3">
            {request.map((option) => (
              <OptionRow key={option.name} option={option} value={values[option.name]} onChange={(value) => void set(option.name, value)} />
            ))}
          </div>
        )}
        {load.length > 0 && (
          <div className="mb-3 ml-[2.75rem] border-l border-[var(--line)] py-1 pl-3">
            <p className="py-1 text-[11px] text-[var(--quiet)]">
              These are sent in the engine's config — they apply on the next server restart.
            </p>
            {load.map((option) => (
              <OptionRow key={option.name} option={option} value={values[option.name]} onChange={(value) => void set(option.name, value)} />
            ))}
          </div>
        )}
        {session.length > 0 && (
          <div className="mb-3 ml-[2.75rem] border-l border-[var(--line)] py-1 pl-3">
            <p className="py-1 text-[11px] text-[var(--quiet)]">
              These ride the engine's session options in its config — they apply on the next server restart.
            </p>
            {session.map((option) => (
              <OptionRow key={option.name} option={option} value={values[option.name]} onChange={(value) => void set(option.name, value)} />
            ))}
          </div>
        )}

        <button
          type="button"
          onClick={onBack}
          className="inline-flex min-h-9 items-center gap-2 rounded-md px-2.5 text-[13px] text-[var(--muted)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
        >
          <ArrowLeft size={14} />
          {backLabel}
        </button>
      </article>
    </>
  );
}

/** One spec option, in the shape the page can hold. */
function OptionRow({
  option,
  value,
  onChange,
}: {
  option: AudioOption;
  value: unknown;
  onChange: (next: unknown) => void;
}) {
  const effective = value === undefined ? option.default : value;
  return (
    <div className="flex min-h-[52px] items-center gap-3 py-1">
      <div className="min-w-0 flex-1">
        <h4 className="truncate text-xs font-medium text-[var(--muted)]">
          {humanize(option.name)}
          {option.required && <span className="ml-1 text-[var(--accent)]">*</span>}
        </h4>
        <p className="truncate text-[11px] text-[var(--quiet)]" title={option.description}>
          {option.description}
        </p>
      </div>
      {option.kind === "bool" ? (
        <input
          type="checkbox"
          checked={effective === true}
          onChange={(event) => onChange(event.target.checked)}
          className="size-4 accent-[var(--accent)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
        />
      ) : option.kind === "enum" ? (
        <select
          value={String(effective ?? "")}
          onChange={(event) => onChange(event.target.value)}
          className="min-h-8 max-w-[220px] rounded-md border border-[var(--line)] bg-[var(--panel)] px-2 text-[12px] text-[var(--text)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
        >
          <option value="">(default)</option>
          {option.values.map((value) => (
            <option key={value} value={value}>
              {value}
            </option>
          ))}
        </select>
      ) : (
        <input
          type={option.kind === "int" || option.kind === "float" ? "number" : "text"}
          step={option.kind === "float" ? "any" : undefined}
          value={effective === null || effective === undefined ? "" : String(effective)}
          placeholder={option.default === null || option.default === undefined ? "" : String(option.default)}
          onChange={(event) => {
            const raw = event.target.value;
            if (option.kind === "int") onChange(raw === "" ? null : Number.parseInt(raw, 10));
            else if (option.kind === "float") onChange(raw === "" ? null : Number.parseFloat(raw));
            else onChange(raw);
          }}
          className="min-h-8 max-w-[220px] rounded-md border border-[var(--line)] bg-[var(--panel)] px-2 text-[12px] text-[var(--text)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
        />
      )}
    </div>
  );
}

/** `voice_design` → `voice design`, for a row a reader scans by eye. */
function humanize(name: string): string {
  return name
    .replace(/_/g, " ")
    .replace(/\b\w/g, (first) => first.toUpperCase());
}

/** Capability tags as a reader reads them. */
function flexibleCap(tag: string): string {
  const words: Record<string, string> = {
    style_control: "style control",
    voice_design: "voice design",
    speaker_reference: "voice cloning",
    built_in_voices: "built-in voices",
    long_form: "long-form",
  };
  return words[tag] ?? humanize(tag);
}

/**
 * One model's cloning setup: the reference clip and what that clip says.
 *
 * The pair is what cloning actually is -- the clip's timbre plus the exact
 * words spoken in it, so the model can align its acoustic prompt. `voice_ref`
 * carries the file's path; `reference_text` the transcript. A family that
 * does not declare `speaker_reference` in its spec never sees this row, which
 * is the point: a file picker over a model that cannot clone is bait.
 */
function CloneVoiceRow({
  values,
  onChange,
  notify,
}: {
  values: Record<string, unknown>;
  onChange: (name: string, next: unknown) => void;
  notify: Notify;
}) {
  const reference = typeof values.voice_ref === "string" ? values.voice_ref : null;
  const fileName = reference?.split(/[\\/]/).pop() ?? null;

  const pick = async () => {
    try {
      const chosen = await openDialog({
        multiple: false,
        filters: [{ name: "Audio clip", extensions: ["wav", "mp3", "flac", "ogg"] }],
      });
      if (typeof chosen === "string") onChange("voice_ref", chosen);
    } catch (reason) {
      notify("error", `The audio clip could not be chosen: ${String(reason)}`);
    }
  };

  return (
    <div className="ml-[2.75rem] border-l border-[var(--line)] py-1 pl-3">
      <div className="flex min-h-[52px] items-center gap-3 py-1">
        <div className="min-w-0 flex-1">
          <h4 className="flex items-center gap-1.5 truncate text-xs font-medium text-[var(--muted)]">
            <AudioLines size={13} className="shrink-0" />
            Clone voice
          </h4>
          <p className="truncate text-[11px] text-[var(--quiet)]">
            A clean 5–15s clip of one speaker, then write below exactly what that clip says.
          </p>
        </div>
        {reference ? (
          <div className="flex shrink-0 items-center gap-1.5">
            <span className="max-w-[180px] truncate text-[12px] text-[var(--text)]" title={reference}>
              {fileName}
            </span>
            <button
              type="button"
              onClick={() => onChange("voice_ref", null)}
              aria-label="Remove the reference clip"
              title="Remove the reference clip"
              className="grid size-7 place-items-center rounded text-[var(--quiet)] hover:bg-[var(--raised)] hover:text-[var(--danger)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
            >
              <X size={13} />
            </button>
          </div>
        ) : (
          <button
            type="button"
            onClick={() => void pick()}
            className="inline-flex min-h-8 shrink-0 items-center gap-1.5 rounded-md border border-[var(--line)] px-2.5 text-[12px] text-[var(--text)] transition-colors hover:border-[var(--line-strong)] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
          >
            Choose clip
          </button>
        )}
      </div>
      <div className="flex min-h-[52px] items-center gap-3 py-1">
        <div className="min-w-0 flex-1">
          <h4 className="truncate text-xs font-medium text-[var(--muted)]">Clip transcript</h4>
          <p className="truncate text-[11px] text-[var(--quiet)]">
            Word for word what the clip says — the model needs the pair.
          </p>
        </div>
        <input
          type="text"
          value={typeof values.reference_text === "string" ? values.reference_text : ""}
          placeholder={reference ? "What the clip says…" : "Choose a clip first"}
          disabled={!reference}
          onChange={(event) => onChange("reference_text", event.target.value)}
          className="min-h-8 max-w-[220px] rounded-md border border-[var(--line)] bg-[var(--panel)] px-2 text-[12px] text-[var(--text)] focus-visible:outline-2 focus-visible:outline-[var(--accent)] disabled:opacity-40"
        />
      </div>
    </div>
  );
}

/** The spec's task words, as the same reader sees them. */
function friendly(task: string): string {
  const words: Record<string, string> = {
    tts: "speech",
    asr: "recognition",
    wake_word: "wake word",
    clone: "voice cloning",
    design: "voice design",
    edit: "editing",
    align: "alignment",
  };
  return words[task] ?? task;
}
