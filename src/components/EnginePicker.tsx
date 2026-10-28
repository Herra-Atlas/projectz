import { invoke } from "@tauri-apps/api/core";
import { useInstalledEngines, type InstalledEngine } from "../features/models/useInstalledEngines";
import type { Notify } from "../features/notifications/types";
import Select, { type SelectOption } from "./Select";

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
    <Select
      value={engineId ?? ""}
      onChange={(value) => void choose(value)}
      label="Engine for this model"
      placeholder="Choose an engine"
      // Only until a choice exists. An empty value that matches no row would
      // render the placeholder, which reads as broken rather than as "not chosen
      // yet"; the disabled row says which it is.
      options={[
        ...(engineId == null ? [{ value: "", label: "Choose an engine", disabled: true }] : []),
        ...(missing ? [{ value: engineId!, label: `${engineId} · not installed` }] : []),
        ...engines.map((engine): SelectOption => ({
          value: engine.id,
          label: `${engine.version}${engine.source === "custom" ? " · unverified" : ""}`,
        })),
      ]}
      className="w-full max-w-[16rem] shrink-0"
    />
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
