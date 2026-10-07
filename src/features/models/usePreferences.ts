import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ModelSelection } from "./types";

/** Settings key holding the Preferences page values. */
export const PREFERENCES_KEY = "app.preferences";

/** Upper bound on the title-model chain. */
export const MAX_TITLE_MODELS = 3;

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
};

const DEFAULT_PREFERENCES: Preferences = { sessionTitleModels: [], subagentModel: null };

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
