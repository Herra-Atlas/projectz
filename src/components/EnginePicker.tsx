import { invoke } from "@tauri-apps/api/core";
import { useInstalledEngines, type InstalledEngine } from "../features/models/useInstalledEngines";
import type { Notify } from "../features/notifications/types";

/**
 * Which engine one local model runs with.
 *
 * Shared by the Local models list and `RuntimeSettingsPage`, because both are
 * describing the same fact about the same model. Two copies would be two
 * `<select>`s whose option lists could differ -- one refreshed, one not -- and a
 * chooser offering an engine the other does not is a way to pick a combination
 * that cannot be represented anywhere.
 *
 * Every model carries its own engine; there is deliberately no global default
 * and no "follow the default" choice. A default would be a second answer to the
 * question this control asks, and the two would disagree the first time one of
 * them changed.
 *
 * Presentational: it reads the engine list, renders the choice, and reports it.
 * The change is applied by the caller through `onChange`, so the two owners each
 * decide what a change means for them -- reloading the model list is
 * `SettingsModal`'s business, not this control's.
 */

type EnginePickerProps = {
  /**
   * The engine this model runs with, or `null` when none has been chosen yet.
   *
   * Passed in rather than read from the engine list so the control shows the same
   * answer the loader would give.
   */
  engineId: string | null | undefined;
  /** Fired after the choice was stored, so the owner can re-read the list. */
  onChange: () => void;
  /** Failures are reported here rather than in a line that scrolls away. */
  notify: Notify;
  /**
   * A key that changes when the model changes.
   *
   * The chooser's state is the selected value, and a page that reuses this
   * component across models would otherwise keep model A's engine showing on
   * model B until something forced a render.
   */
  modelId: string;
};

export default function EnginePicker({ engineId, onChange, notify, modelId }: EnginePickerProps) {
  const engines = useInstalledEngines();
  /**
   * The engine named by the model, if it is one this list knows about.
   *
   * `true` when the model's engine is not installed -- a choice pointing at
   * something since removed. It is still shown rather than replaced with the
   * placeholder, because that choice is exactly what the model will try to load
   * with, and showing "choose an engine" would be a chooser claiming no choice
   * was made when one was. It gets its own entry so the missing state is visible.
   */
  const missing = engineId != null && !engines.some((engine) => engine.id === engineId);

  const choose = async (value: string) => {
    if (!value) return;
    try {
      await invoke("local_model_engine_set", { modelId, engineId: value });
      onChange();
    } catch (reason) {
      notify("error", `Engine could not be changed: ${String(reason)}`, `engine-picker:${modelId}`);
    }
  };

  return (
    <select
      value={engineId ?? ""}
      onChange={(event) => void choose(event.target.value)}
      aria-label="Engine for this model"
      className="h-9 w-full max-w-[16rem] shrink-0 rounded-md border border-[var(--line)] bg-[var(--rail)] px-2 text-xs text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
    >
      {/* Only until a choice exists. A `<select>` with a value matching no option
          renders blank, which reads as broken rather than as "not chosen yet". */}
      {engineId == null && <option value="" disabled>Choose an engine</option>}
      {missing && <option value={engineId!}>{engineId} · not installed</option>}
      {engines.map((engine) => (
        <option key={engine.id} value={engine.id}>
          {engine.version}{engine.source === "custom" ? " · unverified" : ""}
        </option>
      ))}
    </select>
  );
}

/**
 * An engine id rendered as something a person recognises.
 *
 * A group header on the Local models page and a row in a picker both need the
 * same answer -- which build is this -- and a group reading
 * `owner-llama.cpp-prism-b10754-2459f68` would be showing a directory name where
 * a version belongs.
 */
export function engineLabel(engine: Pick<InstalledEngine, "version" | "source"> | undefined): string {
  if (!engine) return "No engine";
  return engine.source === "custom" ? `${engine.version} (unverified)` : engine.version;
}
