import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open as openFile } from "@tauri-apps/plugin-dialog";
import { AlertTriangle, ChevronRight } from "lucide-react";
import { SettingRow, SettingsSection, Toggle } from "./SettingsSection";
import Select from "./Select";
import type { Notify } from "../features/notifications/types";
import { syncArgument, type ArgumentSpec } from "../features/models/arguments";
import { SCALE_STEPS, scaleToValue, valueToScale } from "../features/models/contextScale";
import { formatBytes } from "../features/format/bytes";
import { DEFAULT_CONTEXT, DEFAULT_PARALLEL } from "../features/models/localRuntimeDefaults";

/**
 * Per-model `llama-server` runtime settings.
 *
 * Split out of `SettingsModal` because it is the only page with real state of its
 * own -- an unsaved settings object, a live memory estimate, and a slider that
 * has to be driven from both. Inlined in the modal it was the largest block of
 * logic in the file and the hardest to read in either.
 *
 * ## One rule, repeated everywhere
 *
 * **Off means the flag is not sent.** `llama-server` has its own default for each
 * of these, and several default to `auto` rather than off. Passing an explicit
 * "off" would override a default that was working, so every toggle here maps to
 * `undefined` when off and to a value when on -- the same three-state shape the
 * Rust side stores.
 *
 * ## The layout
 *
 * A dense two-column grid rather than a row per setting. The previous version
 * gave Context and Parallel a full row each, with the label, the description and
 * the control on three separate lines -- two settings took half the page. Here
 * each setting is one line: label and control, with the description only where it
 * is saying something the control's own label cannot. Nine settings fit on a
 * screen that previously held two.
 */

export type KvQuant = "f32" | "f16" | "q8_0" | "q5_1" | "q5_0" | "q4_1" | "q4_0" | "q2_k";

export type LocalRuntimeSettings = {
  context: number;
  parallel: number;
  kv_unified_per_slot?: number | null;
  gpu_layers?: number | null;
  cache_type_k?: KvQuant | null;
  cache_type_v?: KvQuant | null;
  kv_offload?: boolean | null;
  flash_attention?: "auto" | "on" | "off" | null;
  mmproj?: string | null;
  mmproj_offload?: boolean | null;
  chat_template?: string | null;
  chat_template_file?: string | null;
  jinja: boolean;
  temperature?: number | null;
  top_p?: number | null;
  min_p?: number | null;
  repeat_penalty?: number | null;
  mlock: boolean;
  no_mmap: boolean;
  batch_size?: number | null;
  ubatch_size?: number | null;
  draft_model?: string | null;
  draft_gpu_layers?: number | null;
  custom_args?: string | null;
};

/** Templates llama.cpp ships by name, for a GGUF that names none. */
const TEMPLATES = ["chatml", "llama2", "llama3", "mistral", "gemma", "phi3", "qwen2", "deepseek", "vicuna"];

/**
 * How each setting appears in a hand-written argument list.
 *
 * A setting missing from this table is simply not synchronised: it stays in the
 * list exactly as typed. That is the honest failure -- guessing at a flag the
 * build may not have is worse than leaving the text alone.
 */
const ARGUMENT_FLAGS: Partial<Record<keyof LocalRuntimeSettings, ArgumentSpec>> = {
  context: { flag: "--ctx-size" },
  parallel: { flag: "--parallel" },
  gpu_layers: { flag: "-ngl" },
  cache_type_k: { flag: "--cache-type-k" },
  cache_type_v: { flag: "--cache-type-v" },
  kv_offload: { flag: "--no-kv-offload", isSwitch: true },
  flash_attention: { flag: "-fa" },
  chat_template: { flag: "--chat-template" },
  chat_template_file: { flag: "--chat-template-file" },
  jinja: { flag: "--jinja", isSwitch: true },
  temperature: { flag: "--temp" },
  top_p: { flag: "--top-p" },
  min_p: { flag: "--min-p" },
  repeat_penalty: { flag: "--repeat-penalty" },
  mlock: { flag: "--mlock", isSwitch: true },
  no_mmap: { flag: "--no-mmap", isSwitch: true },
  batch_size: { flag: "-b" },
  ubatch_size: { flag: "-ub" },
  draft_model: { flag: "--model-draft" },
  draft_gpu_layers: { flag: "-ngld" },
};

/** What a setting contributes to a custom list, or null when it contributes nothing. */
function argumentValue(spec: ArgumentSpec, value: unknown): string | null {
  if (spec.isSwitch) return value === true ? "" : null;
  if (value === null || value === undefined || value === "") return null;
  return String(value);
}

export type MemoryEstimate = {
  weights_bytes: number;
  kv_bytes: number;
  projector_bytes: number;
  total_bytes: number;
  device_total_bytes?: number | null;
  reliable: boolean;
};

type DeviceMemory = { name: string; total_bytes: number; unified: boolean };

/** Quantisations, best to worst. Order is the order they are offered in. */
const KV_QUANTS: { value: KvQuant; label: string; hint: string }[] = [
  { value: "f32", label: "f32", hint: "No quantisation. Best quality, most memory." },
  { value: "f16", label: "f16", hint: "Half precision. Safe default." },
  { value: "q8_0", label: "q8_0", hint: "Near-f16 quality, a quarter of the memory." },
  { value: "q5_1", label: "q5_1", hint: "Small quality loss." },
  { value: "q5_0", label: "q5_0", hint: "Slightly more loss, smaller still." },
  { value: "q4_1", label: "q4_1", hint: "Noticeable loss." },
  { value: "q4_0", label: "q4_0", hint: "Noticeable loss, slightly smaller." },
  { value: "q2_k", label: "q2_k", hint: "Aggressive. Clear quality cost." },
];

/**
 * Windows' `\\?\` device prefix removed, for display only.
 *
 * It tells the filesystem layer to skip path parsing and means nothing to a
 * person reading a settings row. The stored path keeps it -- that is what the
 * backend opens, and stripping it there would change which file is loaded.
 */
const stripDevicePrefix = (path: string) => path.replace(/^\\\\\?\\/, "");

type RuntimeSettingsProps = {
  modelId: string;
  /**
   * The context window the file declares, in tokens.
   *
   * The slider's ceiling, because that is the only number above which the model
   * cannot actually go: a `-c` past the trained window does not fail, it silently
   * produces answers past the point the model was trained, which is worse than an
   * error. `null` or absent means the header did not report one, and the slider
   * falls back to {@link CONTEXT_FALLBACK_MAX} rather than pretending to know.
   */
  maxContext?: number | null;
  /**
   * The engine this model runs with.
   *
   * Passed in rather than read here because `SettingsModal` already holds the
   * model's row. Reading it from a second place is how a settings page ends up
   * describing an engine the loader would not use.
   */
  engineId?: string | null;
  /** Fired when the settings change, so the parent can persist them. */
  onChange: (settings: LocalRuntimeSettings) => void;
  /** The parent's refresh, so a new engine lands in this model's row. */
  onEngineChanged?: () => void;
  /** Failures are reported here rather than in a line that scrolls away. */
  notify: Notify;
};

/**
 * The ceiling used when a file does not state one.
 *
 * 262144 is llama.cpp's own default `-c`, so it is the honest "no opinion"
 * answer: the process will run at that, so the slider is allowed to reach it.
 */
const CONTEXT_FALLBACK_MAX = 262144;

/**
 * How far the layer track grows at a time, and how close to its end counts as
 * "the top" for the automatic value.
 *
 * Eight layers is the resolution worth choosing -- anything finer is below what
 * affects VRAM -- and reusing it as the top band means the last eighth of the
 * track is the automatic setting rather than the last eighth being a number that
 * happens to mean the same thing.
 */
const LAYER_GROWTH = 8;

/**
 * Where the layer track starts.
 *
 * 128 covers every model in common use, so the realistic range is reachable in
 * one drag. It grew as you passed it for larger models, which meant a 40-layer
 * model asked you to drag past the end four times to reach "all of them".
 */
const INITIAL_LAYER_CEILING = 128;

/**
 * The ceiling this model may actually be run at.
 *
 * The declared window is used as-is, with no rounding to a slider step. An
 * earlier version floored it to a multiple of 512 so the thumb could land on it;
 * the scale now returns `max` exactly at the end of the track, so that
 * constraint is gone and a real figure like 1,000,000 survives intact instead of
 * being reported as 999,424.
 */
const contextCeiling = (declared: number | null | undefined) => {
  if (!declared || declared <= 0) return CONTEXT_FALLBACK_MAX;
  return declared;
};

export default function RuntimeSettings({ modelId, maxContext, onChange }: RuntimeSettingsProps) {
  const [settings, setSettings] = useState<LocalRuntimeSettings | null>(null);
  const [estimate, setEstimate] = useState<MemoryEstimate | null>(null);
  const [device, setDevice] = useState<DeviceMemory | null>(null);
  const [projectorMissing, setProjectorMissing] = useState(false);
  /** The argument list the settings above produce, for the box that shows it. */
  const [generated, setGenerated] = useState("");
  const generatedTimer = useRef<number | null>(null);
  const [advanced, setAdvanced] = useState(false);
  /** Layers to show on the slider. Grown as the user drags past the end. */
  const [layerCeiling, setLayerCeiling] = useState(INITIAL_LAYER_CEILING);

  /**
   * The ceiling this model may actually be run at.
   *
   * Recomputed from the prop rather than read once into state, so a value that
   * arrives after the first render -- the model list is loaded asynchronously, and
   * a model opened straight from disk can report its header a beat later -- moves
   * the ceiling instead of being missed.
   */
  const contextMax = contextCeiling(maxContext);

  // The estimate runs on every keystroke of a slider, and each run reads the
  // model file's GGUF header. Debounced, and the previous result kept until the
  // next arrives -- a figure that blanks while dragging is worse than one that is
  // briefly stale.
  const estimateTimer = useRef<number | null>(null);

  useEffect(() => {
    let mounted = true;
    invoke<LocalRuntimeSettings>("local_runtime_settings", { modelId })
      .then((stored) => {
        if (!mounted) return;
        // Clamped on load, not only on edit. A setting stored before the ceiling
        // was known -- or stored against a different file, since the key is the
        // model's id and a model can be replaced at the same path -- can sit
        // above what this file supports, and a number input will happily display
        // a value its own slider cannot represent. Pulling it down here is silent
        // and immediate; the next edit persists the corrected value.
        setSettings({ ...stored, context: Math.min(stored.context, contextMax) });
      })
      .catch(() => { if (mounted) setSettings({ context: Math.min(DEFAULT_CONTEXT, contextMax), parallel: DEFAULT_PARALLEL, jinja: false, mlock: false, no_mmap: false }); });
    invoke<DeviceMemory | null>("local_device_memory")
      .then((value) => { if (mounted) setDevice(value); })
      .catch(() => undefined);
    return () => { mounted = false; };
  }, [modelId, contextMax]);

  const update = useCallback((change: Partial<LocalRuntimeSettings>) => {
    setSettings((current) => {
      if (!current) return current;
      const next = { ...current, ...change };
      // Once the list has been written by hand it is the truth. A dial then edits
      // the argument it belongs to, instead of quietly discarding the rest of
      // what was typed -- which is what "custom arguments" would otherwise mean
      // the moment anything else on the page was touched.
      if (current.custom_args != null) {
        let text = current.custom_args;
        for (const [key, value] of Object.entries(change)) {
          const spec = ARGUMENT_FLAGS[key as keyof LocalRuntimeSettings];
          if (spec) text = syncArgument(text, spec, argumentValue(spec, value));
        }
        next.custom_args = text.trim() ? text : null;
      }
      onChange(next);
      return next;
    });
  }, [onChange]);

  // The list as the settings above produce it, so the box always shows what
  // would actually be sent rather than a blank that has to be filled in first.
  useEffect(() => {
    if (!settings) return;
    if (generatedTimer.current !== null) window.clearTimeout(generatedTimer.current);
    generatedTimer.current = window.setTimeout(() => {
      invoke<string>("local_runtime_arguments", { settings })
        .then(setGenerated)
        .catch(() => undefined);
    }, 140);
    return () => { if (generatedTimer.current !== null) window.clearTimeout(generatedTimer.current); };
  }, [settings]);

  useEffect(() => {
    if (!settings) return;
    if (estimateTimer.current !== null) window.clearTimeout(estimateTimer.current);
    estimateTimer.current = window.setTimeout(() => {
      invoke<MemoryEstimate>("local_runtime_estimate", { modelId, settings })
        .then((value) => { setEstimate(value); setProjectorMissing(false); })
        .catch(() => undefined);
    }, 140);
    return () => { if (estimateTimer.current !== null) window.clearTimeout(estimateTimer.current); };
  }, [modelId, settings]);

  // A projector file can be moved or deleted after it was chosen, which would
  // otherwise fail at load with a message about a file this page still shows as
  // configured. Surfaced here where it can be fixed.
  useEffect(() => {
    const projector = settings?.mmproj?.trim();
    if (!projector) { setProjectorMissing(false); return; }
    invoke<boolean>("local_path_exists", { path: projector })
      .then((exists) => setProjectorMissing(!exists))
      .catch(() => setProjectorMissing(false));
  }, [settings?.mmproj]);

  const perSlot = useMemo(() => {
    if (!settings) return 0;
    return settings.kv_unified_per_slot
      ? settings.context
      : Math.ceil(settings.context / Math.max(1, settings.parallel));
  }, [settings]);

  const chooseProjector = async () => {
    const selected = await openFile({ multiple: false, filters: [{ name: "Projector", extensions: ["gguf"] }] });
    if (typeof selected !== "string") return;
    update({ mmproj: selected });
  };

  const chooseTemplateFile = async () => {
    const selected = await openFile({
      multiple: false,
      filters: [{ name: "Chat template", extensions: ["jinja", "jinja2", "txt"] }],
    });
    if (typeof selected !== "string") return;
    // The file wins over any text in the box, so the other one is cleared rather
    // than left to look as if it is also in effect.
    update({ chat_template_file: selected, chat_template: null });
  };

  const chooseDraft = async () => {
    const selected = await openFile({
      multiple: false,
      filters: [{ name: "Draft model", extensions: ["gguf"] }],
    });
    if (typeof selected !== "string") return;
    update({ draft_model: selected });
  };

  if (!settings) return <p className="text-sm text-[var(--muted)]">Loading settings…</p>;

  const kvEnabled = settings.cache_type_k != null;

  return (
    <div className="space-y-5">
      {/* The memory figure leads. Everything else on the page is a dial, and this
          is the one number that tells you whether the dial you are turning is
          pointing somewhere survivable. */}
      <MemoryPanel estimate={estimate} device={device} perSlot={perSlot} parallel={settings.parallel} />

      <Group title="Conversation">
        <NumberRow
          label="Context"
          // The ceiling is stated rather than implied, because a thumb that stops
          // at 32K on a 128K model looks broken unless you know it is the model's
          // own limit and not the slider running out of track.
          hint={maxContext
            ? `This model's limit is ${maxContext.toLocaleString()}${settings.kv_unified_per_slot
              ? ` · ${settings.context.toLocaleString()} tokens per conversation`
              : ` · split across ${settings.parallel} slots, ${perSlot.toLocaleString()} each`}`
            : settings.kv_unified_per_slot
              ? `${settings.context.toLocaleString()} tokens per conversation`
              : `Split across ${settings.parallel} slots · ${perSlot.toLocaleString()} each`}
          value={settings.context}
          min={512}
          max={contextMax}
          logarithmic
          onChange={(context) => update({ context })}
        />
        <NumberRow
          label="Parallel conversations"
          hint="How many can run at once"
          value={settings.parallel}
          min={1}
          max={32}
          onChange={(parallel) => update({ parallel })}
        />
        <ToggleRow
          label="Full context per conversation"
          // The limit above is per slot when this is on, which is why a pool
          // larger than the model's limit is not a contradiction -- it is six
          // conversations of that size side by side, and only each one is
          // bounded. Saying so here stops the number reading as a bug.
          hint={settings.kv_unified_per_slot
            ? `Each slot gets all ${settings.context.toLocaleString()} tokens${maxContext ? ` (this model's limit)` : ""} — the pool grows to ${(settings.context * settings.parallel).toLocaleString()}`
            : `Off: all ${settings.parallel} slots share ${settings.context.toLocaleString()} tokens`}
          checked={settings.kv_unified_per_slot != null}
          onChange={(on) => update({ kv_unified_per_slot: on ? settings.context : null })}
        />
      </Group>

      <Group title="KV cache">
        <ToggleRow
          label="Quantise the cache"
          hint="Smaller cache, some quality cost"
          checked={kvEnabled}
          onChange={(on) => update(on
            ? { cache_type_k: settings.cache_type_k ?? "q8_0", cache_type_v: settings.cache_type_v ?? "q8_0" }
            : { cache_type_k: null, cache_type_v: null })}
        />
        {kvEnabled && (
          <>
            {/* K and V are two flags and two columns, and V tolerates more
                quantisation than K. One control for both would remove a choice
                that is genuinely worth making, so they stay separate. */}
            <SelectRow
              label="K cache"
              hint="What attention scores compare against. Keep this higher than V."
              value={settings.cache_type_k ?? "q8_0"}
              options={KV_QUANTS.map(({ value, label }) => ({ value, label }))}
              onChange={(value) => update({ cache_type_k: value as KvQuant })}
            />
            <SelectRow
              label="V cache"
              hint="Attention values. Handles aggressive quantisation better than K."
              value={settings.cache_type_v ?? "q8_0"}
              options={KV_QUANTS.map(({ value, label }) => ({ value, label }))}
              onChange={(value) => update({ cache_type_v: value as KvQuant })}
            />
          </>
        )}
      </Group>

      <Group title="Vision">
        <SettingRow
            label="Projector"
            description={projectorMissing
              ? "File not found — pick it again"
              : settings.mmproj ? stripDevicePrefix(settings.mmproj) : "No projector. This model cannot read images."}
            control={
              <div className="flex items-center gap-2">
                {settings.mmproj && (
                  <button type="button" onClick={() => update({ mmproj: null, mmproj_offload: null })} className="h-9 rounded-md px-2.5 text-xs text-[var(--quiet)] hover:bg-[var(--raised)] hover:text-[var(--text)]">Clear</button>
                )}
                <button type="button" onClick={() => void chooseProjector()} className="h-9 rounded-md border border-[var(--line)] px-3 text-xs text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]">
                  {settings.mmproj ? "Change" : "Choose file"}
                </button>
              </div>
            }
          />
        {settings.mmproj && (
          <ToggleRow
            label="Projector on GPU"
            hint={settings.mmproj_offload == null ? "llama.cpp decides" : settings.mmproj_offload ? "Runs in VRAM" : "Runs on CPU"}
            checked={settings.mmproj_offload === true}
            onChange={(on) => update({ mmproj_offload: on ? true : null })}
            triState={settings.mmproj_offload == null}
          />
        )}
      </Group>

      <button
        type="button"
        onClick={() => setAdvanced((open) => !open)}
        aria-expanded={advanced}
        className="flex w-full items-center gap-1.5 rounded-md px-1 py-1 text-left text-[12px] text-[var(--muted)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
      >
        <ChevronRight size={13} className={`shrink-0 transition-transform ${advanced ? "rotate-90" : ""}`} />
        Advanced
      </button>

      {advanced && (
        <>
      <Group title="GPU">
        <SliderRow
          label="Layers on GPU"
          hint={settings.gpu_layers === 999 || settings.gpu_layers == null
            ? "As many as fit — llama.cpp decides"
            : `${settings.gpu_layers} of the model's layers`}
          value={settings.gpu_layers ?? 999}
          min={0}
          max={layerCeiling}
          onChange={(gpu_layers) => {
            // Dragging to the very top means "as many as fit", which is what
            // llama.cpp is asked for with 999. Mapping the last eighth of the
            // track to it means the top of the range is the automatic value, so
            // the control reads as one continuous scale rather than a number with
            // a hidden mode behind it -- and it still cannot be dragged past its
            // own end, which is what a bare 999 on the track could do.
            const atTop = gpu_layers >= layerCeiling - LAYER_GROWTH;
            setLayerCeiling((current) => (gpu_layers >= current
              ? Math.ceil((gpu_layers + LAYER_GROWTH) / LAYER_GROWTH) * LAYER_GROWTH
              : current));
            update({ gpu_layers: atTop ? 999 : gpu_layers });
          }}
          onAuto={() => update({ gpu_layers: 999 })}
          autoActive={settings.gpu_layers === 999}
        />
        <ToggleRow
          label="KV cache on GPU"
          hint={settings.kv_offload == null ? "llama.cpp decides" : settings.kv_offload ? "Kept in VRAM" : "Kept in system memory"}
          checked={settings.kv_offload === true}
          onChange={(on) => update({ kv_offload: on ? true : null })}
          triState={settings.kv_offload == null}
        />
        <SelectRow
          label="Flash attention"
          value={settings.flash_attention ?? "auto"}
          options={[
            { value: "auto", label: "Auto" },
            { value: "on", label: "On" },
            { value: "off", label: "Off" },
          ]}
          onChange={(value) => update({ flash_attention: value as LocalRuntimeSettings["flash_attention"] })}
        />
      </Group>

      {/* A wrong template is the commonest reason a local model answers nonsense,
          so it leads the sections about how the model is spoken to. */}
      <Group title="Chat template">
        <SelectRow
          label="Template"
          hint={
            settings.chat_template_file
              ? "Using the chosen .jinja file"
              : settings.chat_template == null
                ? "Whatever the GGUF declares"
                : TEMPLATES.includes(settings.chat_template)
                  ? "Built into llama.cpp"
                  : "Your own template"
          }
          value={templateSelection(settings)}
          options={[
            { value: "", label: "From the model" },
            ...TEMPLATES.map((name) => ({ value: name, label: name })),
            { value: "custom", label: "Custom…" },
          ]}
          onChange={(value) =>
            update({
              chat_template: value === "" ? null : value === "custom" ? (settings.chat_template ?? "") : value,
              chat_template_file: null,
            })
          }
        />
        {templateSelection(settings) === "custom" && (
          <DraftRow
            label="Template text"
            description="The Jinja template itself. Applies to this model only."
            value={settings.chat_template ?? ""}
            multiline
            placeholder="{{ messages }}"
            onCommit={(chat_template) => update({ chat_template: chat_template.trim() ? chat_template : null })}
          />
        )}
        <SettingRow
          label="From a file"
          description={settings.chat_template_file ? stripDevicePrefix(settings.chat_template_file) : "A .jinja file kept outside the app"}
          control={
            <div className="flex items-center gap-2">
              {settings.chat_template_file && (
                <button type="button" onClick={() => update({ chat_template_file: null })} className="h-9 rounded-md px-2.5 text-xs text-[var(--quiet)] hover:bg-[var(--raised)] hover:text-[var(--text)]">Clear</button>
              )}
              <button type="button" onClick={() => void chooseTemplateFile()} className="h-9 rounded-md border border-[var(--line)] px-3 text-xs text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]">
                {settings.chat_template_file ? "Change" : "Choose file"}
              </button>
            </div>
          }
        />
        <ToggleRow
          label="Jinja engine"
          hint="Needed by some templates, and by tool calling on them"
          checked={settings.jinja}
          onChange={(jinja) => update({ jinja })}
        />
      </Group>

      <Group title="Generation">
        <DecimalRow label="Temperature" hint="Higher is more varied" value={settings.temperature} min={0} max={2} step={0.05} onChange={(temperature) => update({ temperature })} />
        <DecimalRow label="Top-p" hint="Nucleus sampling cutoff" value={settings.top_p} min={0} max={1} step={0.01} onChange={(top_p) => update({ top_p })} />
        <DecimalRow label="Min-p" hint="Drops tokens below this probability" value={settings.min_p} min={0} max={1} step={0.01} onChange={(min_p) => update({ min_p })} />
        <DecimalRow label="Repeat penalty" hint="Penalises repetition over 1" value={settings.repeat_penalty} min={0.5} max={2} step={0.01} onChange={(repeat_penalty) => update({ repeat_penalty })} />
      </Group>

      <Group title="Memory">
        <ToggleRow
          label="Lock in RAM"
          hint="Stops the model being paged out; fails to load if there is not enough"
          checked={settings.mlock}
          onChange={(mlock) => update({ mlock })}
        />
        <ToggleRow
          label="Read without mapping"
          hint="Slower to start, steady when the file is on a slow disk"
          checked={settings.no_mmap}
          onChange={(no_mmap) => update({ no_mmap })}
        />
      </Group>

      <Group title="Batch">
        <IntegerRow label="Prompt batch" hint="Tokens processed per step. Larger is faster, needs more memory." value={settings.batch_size} min={32} max={8192} onChange={(batch_size) => update({ batch_size })} />
        <IntegerRow label="GPU batch" hint="The smaller batch the GPU actually sees" value={settings.ubatch_size} min={16} max={4096} onChange={(ubatch_size) => update({ ubatch_size })} />
      </Group>

      <Group title="Speculative decoding">
        <SettingRow
          label="Draft model"
          description={settings.draft_model ? stripDevicePrefix(settings.draft_model) : "A small model that proposes tokens for this one to accept"}
          control={
            <div className="flex items-center gap-2">
              {settings.draft_model && (
                <button type="button" onClick={() => update({ draft_model: null, draft_gpu_layers: null })} className="h-9 rounded-md px-2.5 text-xs text-[var(--quiet)] hover:bg-[var(--raised)] hover:text-[var(--text)]">Clear</button>
              )}
              <button type="button" onClick={() => void chooseDraft()} className="h-9 rounded-md border border-[var(--line)] px-3 text-xs text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]">
                {settings.draft_model ? "Change" : "Choose file"}
              </button>
            </div>
          }
        />
        {settings.draft_model && (
          <IntegerRow label="Draft on GPU" hint="Layers of the draft model to offload" value={settings.draft_gpu_layers} min={0} max={999} onChange={(draft_gpu_layers) => update({ draft_gpu_layers })} />
        )}
      </Group>

      {/* The list as it will actually be sent. Typing in it takes it over; from
          then on the dials above edit their own flag inside it. */}
      <Group title="Startup arguments">
        <DraftRow
          label="Arguments"
          description={
            settings.custom_args
              ? "Edited by hand. Changing a setting above edits its flag here. --model, --alias, --host and --port stay with the app."
              : "Exactly what will be sent. Type here to take it over; a setting changed afterwards edits its own flag."
          }
          value={settings.custom_args ?? generated}
          multiline
          placeholder="--ctx-size 30000 --parallel 1"
          onCommit={(text) => update({ custom_args: text.trim() ? text : null })}
        />
        {settings.custom_args && (
          <SettingRow
            label="Custom list"
            description="Go back to the arguments the settings above produce"
            control={
              <button type="button" onClick={() => update({ custom_args: null })} className="h-9 rounded-md border border-[var(--line)] px-3 text-xs text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]">Clear</button>
            }
          />
        )}
      </Group>
        </>
      )}
    </div>
  );
}

/** Which option the template picker shows for the stored value. */
function templateSelection(settings: LocalRuntimeSettings): string {
  // Checked for null rather than falsiness: choosing "Custom" stores an empty
  // string, and reading that as "nothing chosen" would bounce the picker back to
  // "From the model" the moment it was chosen.
  if (settings.chat_template == null) return "";
  return TEMPLATES.includes(settings.chat_template) ? settings.chat_template : "custom";
}

/** A text field that commits on blur rather than on every keystroke. */
function DraftRow({
  label,
  description,
  value,
  placeholder,
  multiline = false,
  onCommit,
}: {
  label: string;
  description: string;
  value: string;
  placeholder?: string;
  multiline?: boolean;
  onCommit: (value: string) => void;
}) {
  const [draft, setDraft] = useState(value);
  useEffect(() => {
    setDraft(value);
  }, [value]);
  const commit = () => {
    if (draft !== value) onCommit(draft);
  };
  const shared = "w-full rounded-md border border-[var(--line)] bg-[var(--rail)] px-2 py-1.5 font-mono text-[11px] text-[var(--text)] outline-none focus-visible:border-[var(--accent)]";
  return (
    <SettingRow
      label={label}
      description={description}
      control={
        multiline ? (
          <textarea
            value={draft}
            rows={4}
            placeholder={placeholder}
            onChange={(event) => setDraft(event.target.value)}
            onBlur={commit}
            className={`${shared} min-w-[18rem] resize-y`}
          />
        ) : (
          <input
            value={draft}
            placeholder={placeholder}
            onChange={(event) => setDraft(event.target.value)}
            onBlur={commit}
            onKeyDown={(event) => {
              if (event.key === "Enter") event.currentTarget.blur();
            }}
            className={`${shared} min-w-[14rem]`}
          />
        )
      }
    />
  );
}

/** A whole number that commits on blur, with an empty value meaning "default". */
function IntegerRow({
  label,
  hint,
  value,
  min,
  max,
  onChange,
}: {
  label: string;
  hint: string;
  value?: number | null;
  min: number;
  max: number;
  onChange: (value: number | null) => void;
}) {
  const [draft, setDraft] = useState(value == null ? "" : String(value));
  useEffect(() => {
    setDraft(value == null ? "" : String(value));
  }, [value]);
  const commit = () => {
    if (!draft.trim()) {
      if (value != null) onChange(null);
      return;
    }
    const parsed = Number(draft);
    if (!Number.isFinite(parsed)) {
      setDraft(value == null ? "" : String(value));
      return;
    }
    onChange(Math.min(max, Math.max(min, Math.round(parsed))));
  };
  return (
    <SettingRow
      label={label}
      description={hint}
      control={
        <input
          type="number"
          min={min}
          max={max}
          value={draft}
          placeholder="default"
          onChange={(event) => setDraft(event.target.value)}
          onBlur={commit}
          onKeyDown={(event) => {
            if (event.key === "Enter") event.currentTarget.blur();
          }}
          className="h-9 w-28 rounded-md border border-[var(--line)] bg-[var(--rail)] px-2 text-sm tabular-nums text-[var(--text)] outline-none focus-visible:border-[var(--accent)]"
        />
      }
    />
  );
}

/** A decimal that commits on blur, with an empty value meaning "default". */
function DecimalRow({
  label,
  hint,
  value,
  min,
  max,
  step,
  onChange,
}: {
  label: string;
  hint: string;
  value?: number | null;
  min: number;
  max: number;
  step: number;
  onChange: (value: number | null) => void;
}) {
  const [draft, setDraft] = useState(value == null ? "" : String(value));
  useEffect(() => {
    setDraft(value == null ? "" : String(value));
  }, [value]);
  const commit = () => {
    if (!draft.trim()) {
      if (value != null) onChange(null);
      return;
    }
    const parsed = Number(draft);
    if (!Number.isFinite(parsed)) {
      setDraft(value == null ? "" : String(value));
      return;
    }
    onChange(Math.min(max, Math.max(min, parsed)));
  };
  return (
    <SettingRow
      label={label}
      description={hint}
      control={
        <input
          type="number"
          min={min}
          max={max}
          step={step}
          value={draft}
          placeholder="default"
          onChange={(event) => setDraft(event.target.value)}
          onBlur={commit}
          onKeyDown={(event) => {
            if (event.key === "Enter") event.currentTarget.blur();
          }}
          className="h-9 w-28 rounded-md border border-[var(--line)] bg-[var(--rail)] px-2 text-sm tabular-nums text-[var(--text)] outline-none focus-visible:border-[var(--accent)]"
        />
      }
    />
  );
}

/**
 * The memory estimate, and what it does and does not claim.
 *
 * An estimate, and it says so in the words on screen rather than in a tooltip
 * nobody opens. llama.cpp also allocates for activations and scratch space,
 * which are not knowable before a load, so this is the part that is predictable:
 * weights, KV cache, projector.
 */
function MemoryPanel({
  estimate,
  device,
  perSlot,
  parallel,
}: {
  estimate: MemoryEstimate | null;
  device: DeviceMemory | null;
  perSlot: number;
  parallel: number;
}) {
  if (!estimate) {
    return <div className="rounded-lg border border-[var(--line)] bg-[var(--rail)] px-3.5 py-3 text-[11px] text-[var(--quiet)]">Estimating…</div>;
  }
  const exceeds = estimate.device_total_bytes != null && estimate.total_bytes > estimate.device_total_bytes;
  const tight = estimate.device_total_bytes != null && !exceeds && estimate.total_bytes > estimate.device_total_bytes * 0.85;

  return (
    <div className="rounded-lg border border-[var(--line)] bg-[var(--rail)] px-3.5 py-3">
      <div className="flex items-baseline justify-between gap-3">
        <div className="flex items-baseline gap-2">
          <span className="text-lg font-medium tabular-nums text-[var(--text)]">{formatBytes(estimate.total_bytes)}</span>
          <span className="text-[11px] text-[var(--quiet)]">estimated VRAM</span>
        </div>
        {exceeds && (
          <span className="inline-flex items-center gap-1 text-[11px] text-[var(--danger)]">
            <AlertTriangle size={12} />Over {device ? `${formatBytes(estimate.device_total_bytes!)}` : "available memory"}
          </span>
        )}
        {tight && !exceeds && (
          <span className="text-[11px] text-[var(--muted)]">Tight for {device ? formatBytes(estimate.device_total_bytes!) : "this device"}</span>
        )}
        {!exceeds && !tight && device && (
          <span className="text-[11px] text-[var(--quiet)]">of {formatBytes(device.total_bytes)}</span>
        )}
      </div>

      <dl className="mt-2 grid grid-cols-3 gap-2 text-[11px]">
        <Figure label="Weights" value={formatBytes(estimate.weights_bytes)} />
        <Figure
          label="KV cache"
          value={estimate.reliable ? formatBytes(estimate.kv_bytes) : "unknown"}
          hint={estimate.reliable ? `${perSlot.toLocaleString()} × ${parallel} slots` : "This file's header could not be read"}
        />
        <Figure label="Projector" value={estimate.projector_bytes > 0 ? formatBytes(estimate.projector_bytes) : "—"} />
      </dl>
    </div>
  );
}

function Figure({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return (
    <div title={hint}>
      <dt className="text-[var(--quiet)]">{label}</dt>
      <dd className="tabular-nums text-[var(--muted)]">{value}</dd>
    </div>
  );
}

/**
 * A labelled category.
 *
 * Delegates to `SettingsSection` rather than reimplementing it, so this page and
 * every other Settings page share one heading and one divider. Matching matters
 * more than it looks -- a boxed group here and a ruled section on the next page
 * reads as two different apps.
 */
function Group({ title, children }: { title: string; children: React.ReactNode }) {
  return <SettingsSection title={title}>{children}</SettingsSection>;
}

/**
 * A number field.
 *
 * A slider as well as the input, because a context of 32768 is not something
 * anyone types by hand and the range matters. The input stays because the useful
 * values are not round numbers.
 *
 * `logarithmic` switches the track to the ratio scale in
 * `features/models/contextScale`. Only for the fields whose range spans orders
 * of magnitude -- context does, `parallel` does not, so that one is left linear.
 */
function NumberRow({
  label, hint, value, min, max, step, logarithmic = false, onChange,
}: {
  label: string;
  hint: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  logarithmic?: boolean;
  onChange: (value: number) => void;
}) {
  // `step` is 1 for a plain number field. The logarithmic track ignores it and
  // rounds to significant digits instead, because a fixed step is incompatible
  // with a log scale -- see `contextScale.ts`.
  const stepSize = step ?? 1;

  return (
    <SettingRow
      label={label}
      description={hint}
      control={
        <div className="flex items-center gap-3">
          {logarithmic ? (
            // The range input carries positions, not values, so the `aria`
            // label has to name what the number means rather than the units the
            // underlying element happens to use.
            <input
              type="range"
              min={0}
              max={SCALE_STEPS}
              step={1}
              value={valueToScale(value, min, max)}
              onChange={(event) => onChange(scaleToValue(Number(event.target.value), min, max))}
              aria-label={`${label} slider, ${min.toLocaleString()} to ${max.toLocaleString()}`}
              aria-valuetext={`${value.toLocaleString()}`}
              className="h-1 w-28 cursor-pointer appearance-none rounded-full bg-[var(--line)] accent-[var(--accent)]"
            />
          ) : (
            <input
              type="range"
              min={min}
              max={max}
              step={stepSize}
              value={value}
              onChange={(event) => onChange(Number(event.target.value))}
              aria-label={`${label} slider`}
              className="h-1 w-28 cursor-pointer appearance-none rounded-full bg-[var(--line)] accent-[var(--accent)]"
            />
          )}
          <input
            type="number"
            min={min}
            max={max}
            value={value}
            onChange={(event) => onChange(Math.max(min, Math.min(max, Number(event.target.value) || min)))}
            aria-label={label}
            className="h-9 w-24 shrink-0 rounded-md border border-[var(--line)] bg-[var(--rail)] px-2 text-right text-sm tabular-nums text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
          />
        </div>
      }
    />
  );
}

/**
 * A slider whose top value is "as many as possible".
 *
 * `auto` is a distinct answer rather than a number, so it is shown as a label
 * rather than stored as 999 -- and it is shown *without* disabling the track.
 * That earlier version greyed the slider out whenever auto was active, which
 * combined with 999 being the default meant the control was unusable until you
 * found the one button that turned auto off. So the track stays live: automatic
 * parks the thumb at the top, and dragging it left off that end is what selects
 * an explicit count. Leaving it alone changes nothing; touching it opts you out.
 */
function SliderRow({
  label, hint, value, min, max, onChange, onAuto, autoActive,
}: {
  label: string;
  hint: string;
  value: number;
  min: number;
  max: number;
  onChange: (value: number) => void;
  onAuto: () => void;
  autoActive: boolean;
}) {
  return (
    <SettingRow
      label={label}
      description={hint}
      control={
        <div className="flex items-center gap-3">
          {/**
            Never disabled, and that is the whole point of this control.
          */}
          <input
            type="range"
            min={min}
            max={max}
            // Auto parks the thumb at the far right rather than at a magic
            // number off the end of the track, so "as many as fit" reads as the
            // top of the range rather than as an error state. Dragging left off
            // that end is what switches to an explicit count -- so the way out
            // of automatic is the track itself, not a button you have to find.
            value={autoActive ? max : value}
            onChange={(event) => onChange(Number(event.target.value))}
            aria-label={label}
            aria-valuetext={autoActive ? "Automatic" : `${value} layers`}
            className="h-1 w-28 cursor-pointer appearance-none rounded-full bg-[var(--line)] accent-[var(--accent)]"
          />
          <button
            type="button"
            onClick={onAuto}
            aria-pressed={autoActive}
            title={autoActive ? "Automatic — click to set a number" : "Let llama.cpp decide"}
            className={`h-9 w-[4.5rem] shrink-0 rounded-md border px-1.5 text-xs tabular-nums transition-colors ${autoActive
              ? "border-[var(--accent)] text-[var(--accent)]"
              : "border-[var(--line)] text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]"}`}
          >
            {autoActive ? "Auto" : value}
          </button>
        </div>
      }
    />
  );
}

/**
 * An on/off switch, on the shared `Toggle` every other settings page uses.
 *
 * `triState` distinguishes "explicitly off" from "not set", because for these
 * settings those are different things and only one of them is a choice the user
 * made -- see the note on `LocalRuntimeSettings`. Without it a control cannot
 * express the default it is overriding.
 */
function ToggleRow({
  label, hint, checked, onChange, triState = false,
}: {
  label: string;
  hint: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  triState?: boolean;
}) {
  return (
    <SettingRow
      label={label}
      description={hint}
      control={
        <div className="flex items-center gap-2">
          {triState && <span className="text-xs text-[var(--quiet)]">Default</span>}
          <Toggle checked={checked} onChange={onChange} label={label} />
        </div>
      }
    />
  );
}

function SelectRow({
  label, hint, value, options, onChange,
}: {
  label: string;
  hint?: string;
  value: string;
  options: { value: string; label: string; hint?: string }[];
  onChange: (value: string) => void;
}) {
  return (
    <SettingRow
      label={label}
      description={hint}
      control={
        <Select
          value={value}
          options={options}
          onChange={onChange}
          label={label}
          size="sm"
          className="w-32 shrink-0"
        />
      }
    />
  );
}