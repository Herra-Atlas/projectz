import { Segmented } from "../../SettingsSection";
import type { Preferences } from "../../../features/models/usePreferences";
import type { ModelGroup } from "../../../features/models/useModelRegistry";
import type { LocalModel } from "../../../features/models/types";

/**
 * What every page of the Model section is handed by the settings modal.
 *
 * One type rather than four copies, because the four pages are one form split
 * across nav entries — they all edit the same `Preferences` record, and a type
 * shared here is what keeps a page from inventing its own prop names and
 * drifting from how the modal calls them.
 *
 * `onChange` receives the *whole* next record rather than a partial: the caller
 * (the modal, via `usePreferences`) persists it, and a partial would have to be
 * merged somewhere anyway — better once, here, than at every call site.
 *
 * `groups` arrives from the outside so the pickers cannot offer a different set
 * of models than the chat picker does; it is the modal's own
 * `providerModelGroups` (enabled providers, their enabled models).
 */
export type ModelPageProps = {
  preferences: Preferences;
  /** Hand the next record upstream; the caller owns persistence. */
  onChange: (next: Preferences) => void;
  /** Enabled providers and their models, in the shape the pickers want. */
  groups: ModelGroup[];
  /** Locally installed GGUF models, alongside the provider models. */
  localModels: LocalModel[];
};

/**
 * The reasoning effort, from what is sent to the provider to what the row shows.
 *
 * Stored as the raw wire string (`""` included, which means "provider default")
 * because it is sent verbatim to the provider. The picker shows words; the
 * stored value is the wire name mapped back from the label on choose — the same
 * round trip `Preferences` always went through.
 */
export const REASONING_CHOICES = [
  { label: "Provider default", value: "" },
  { label: "Low", value: "low" },
  { label: "Medium", value: "medium" },
  { label: "High", value: "high" },
] as const;

/** The label nearest a stored effort, falling back to "Provider default" for
 *  anything the provider invents and the backend forwards. */
export function reasoningLabel(value: string): string {
  return REASONING_CHOICES.find((choice) => choice.value === value)?.label ?? REASONING_CHOICES[0].label;
}

/** The wire value behind a label the user pressed. */
export function reasoningValue(label: string): string {
  return REASONING_CHOICES.find((choice) => choice.label === label)?.value ?? "";
}

/** Effort as one `Segmented`, for a `Preferences` record. Shared because the
 *  main model and the sub-agents speak the same vocabulary — producing them
 *  separately is how the two pages would end up with four options one and none
 *  the other. */
export function ReasoningSegmented({ value, onChange, label }: { value: string; onChange: (next: string) => void; label: string }) {
  return (
    <Segmented
      label={label}
      value={reasoningLabel(value)}
      options={REASONING_CHOICES.map((choice) => choice.label)}
      onChange={(label) => onChange(reasoningValue(label))}
    />
  );
}

/**
 * A number owned by the whole preferences record.
 *
 * Controlled straight against the record, writing `Number(value) || 0` — an
 * empty field, the button artefact or a plainly wrong typed value lands as 0
 * rather than stalling the picker mid-gesture, and `usePreferences` clamps the
 * stored record against its own bounds on the next read.
 *
 * The blur re-commits `min` when the field is below it (covers the negative
 * text state the `|| 0` path cannot produce on a partial type). Fractional
 * fields pass `step: "any"` or a decimal step; the input's own arrows keep to
 * it, the typed value is free until the clamp.
 */
export function NumberInput({
  value,
  min = 0,
  max,
  step = 1,
  label,
  disabled,
  onChange,
}: {
  value: number;
  min?: number;
  max?: number;
  step?: number;
  label: string;
  disabled?: boolean;
  onChange: (next: number) => void;
}) {
  return (
    <input
      type="number"
      inputMode="numeric"
      aria-label={label}
      value={value}
      min={min}
      max={max}
      step={step}
      disabled={disabled}
      onChange={(event) => onChange(Number(event.target.value) || 0)}
      onBlur={() => {
        if (value < min) onChange(min);
        else if (max !== undefined && value > max) onChange(max);
      }}
      className="h-9 w-[92px] rounded-md border border-[var(--line)] bg-[var(--rail)] px-2.5 text-[13px] text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)] disabled:cursor-not-allowed disabled:opacity-40"
    />
  );
}
