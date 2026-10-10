import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ModelSelection } from "./types";

/** Settings key holding the Preferences page values. */
export const PREFERENCES_KEY = "app.preferences";

/** Upper bound on the title-model chain. */
export const MAX_TITLE_MODELS = 3;

/** How aggressively old context is elided before a request is sent.
 *
 * Wire names match `Compaction` in Rust (`ai/compact.rs`). An unknown or absent
 * value deserializes to `normal` there, which is the safe default: losing some
 * old tool output is never as bad as a request the provider refuses for being
 * too long. */
export type Compaction = "off" | "normal" | "fast";

/** Human labels for the compaction selector, in the order they are offered. */
export const COMPACTION_LABELS: Record<Compaction, string> = {
  off: "Off",
  normal: "Normal",
  fast: "Fast",
};

/** True when a value is a compaction level the backend accepts. */
export function isCompaction(value: unknown): value is Compaction {
  return value === "off" || value === "normal" || value === "fast";
}

/** Reads a stored compaction level, falling back to `normal`. */
export function compactionOrDefault(value: unknown): Compaction {
  return isCompaction(value) ? value : "normal";
}

export type Preferences = {
  /**
   * Models tried in order when naming a new conversation. The first entry is
   * the primary; later entries are fallbacks used only when an earlier one
   * cannot answer (for example a local model whose engine is not running).
   * An empty list switches the feature off.
   */
  sessionTitleModels: ModelSelection[];
  /**
   * The model sub-agents run on when their parent does not name one.
   *
   * A single selection, not a chain: a sub-agent is one request, so there is no
   * fallback to try. `null` means "use whatever model the parent is using", which
   * is the honest default — a sub-agent on the same model it was spawned from
   * needs no setup.
   *
   * Same shape as a title entry, so a remote `{endpointId, model}` and a local
   * `{localModelId}` both fit and both resolve through the same backend path.
   */
  subagentModel: ModelSelection | null;
  /**
   * The model an image is shown to when `read_file` opens one.
   *
   * Same selection shape as the sub-agent model, and the same `null` meaning: no
   * vision model, so reading an image is a clear refusal rather than a silent
   * attempt at a model that cannot see.
   */
  visionModel: ModelSelection | null;
  /**
   * How aggressively old context is elided before a request is sent.
   *
   * Read by the backend at the start of each run; `normal` elides old tool
   * output, `fast` also drops older turns.
   */
  compaction: Compaction;
};

const DEFAULT_PREFERENCES: Preferences = {
  sessionTitleModels: [],
  subagentModel: null,
  visionModel: null,
  compaction: "normal",
};

/**
 * Loads `app.preferences` once per enabled run and exposes a setter that
 * persists through `database_set_setting`. Shared so the chat view and the
 * settings modal agree on the key and shape.
 */
export function usePreferences(enabled: boolean) {
  const [preferences, setPreferences] = useState<Preferences>(DEFAULT_PREFERENCES);

  useEffect(() => {
    if (!enabled) return;
    let mounted = true;
    invoke<Partial<Preferences> | null>("database_get_setting", { key: PREFERENCES_KEY })
      .then((saved) => {
        if (!mounted) return;
        // Migrate the original single-model shape so an existing choice is kept
        // rather than silently discarded.
        const legacy = (saved as { sessionTitleModel?: ModelSelection | null } | null)?.sessionTitleModel;
        const models = saved?.sessionTitleModels ?? (legacy ? [legacy] : []);
        setPreferences({
          sessionTitleModels: models.slice(0, MAX_TITLE_MODELS),
          // Absent on a record written before sub-agents existed, which reads as
          // "same as the parent" rather than failing.
          subagentModel: saved?.subagentModel ?? null,
          // Absent on a record written before either existed: no vision model
          // (images refuse cleanly) and normal compaction.
          visionModel: saved?.visionModel ?? null,
          compaction: compactionOrDefault(saved?.compaction),
        });
      })
      .catch(() => undefined);
    return () => { mounted = false; };
  }, [enabled]);

  const savePreferences = useCallback((next: Preferences) => {
    setPreferences(next);
    void invoke("database_set_setting", { key: PREFERENCES_KEY, value: next })
      .catch((reason: unknown) => {
        // Surface the failure but keep the optimistic local value so the
        // picker still reflects the user's last choice.
        console.error("Preferences could not be saved:", reason);
      });
  }, []);

  return { preferences, savePreferences };
}
