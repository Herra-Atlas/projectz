import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ModelSelection } from "./types";

/** Settings key holding the Model section's values. Shared with the backend,
 *  which reads `app.preferences` for compaction and runtime limits. */
export const PREFERENCES_KEY = "app.preferences";

/** Upper bound on the title-model chain. */
export const MAX_TITLE_MODELS = 3;

/** Upper bound on the chain that answers messages from a chat app.
 *
 * The same number as titles because it is the same idea: the first model that can
 * answer wins, and three is enough to survive one provider being down without
 * turning a phone message into a tour of every model installed. */
export const MAX_MESSAGING_MODELS = 3;

/** Defaults that the backend reads the same way, so both sides agree about what
 *  "unset" means. Kept in one place and exported, because the settings pages'
 *  number inputs clamp against them. */
export const DEFAULT_PREFERENCES = {
  /** Tool rounds a main reply may spend. The old Rust constant was 100. */
  mainSteps: 150,
  /** Tool rounds a sub-agent may spend. */
  subagentSteps: 100,
  /** Sub-agents allowed running at once. 0 means no ceiling. */
  subagentParallel: 5,
  /** Auto-compression on, trigger at 80% of a measured window, rewriting the
   *  older half of the conversation, aiming at 20% of the window, keeping the
   *  last 20 messages verbatim, and giving a summarising model two minutes. */
  autoCompact: true,
  compactAtPct: 80,
  compactReach: 0.5,
  compactTarget: 0.2,
  compactProtect: 20,
  compactTimeout: 120,
} as const;

export type Preferences = {
  /**
   * Models tried in order when naming a new conversation. The first entry is
   * the primary; later entries are fallbacks used only when an earlier one
   * cannot answer (for example a local model whose engine is not running).
   * An empty list switches the feature off.
   */
  sessionTitleModels: ModelSelection[];
  /**
   * Models tried in order when a message arrives from a chat app.
   *
   * The same chain shape as titles, and for the same reason: a provider being down
   * should cost a retry, not the feature. Empty switches messaging off by name rather
   * than guessing a model nobody chose.
   */
  messagingModels: ModelSelection[];
  /**
   * The model sub-agents run on when their parent does not name one.
   *
   * A single selection: a sub-agent is one request, so there is no fallback
   * chain. `null` means "use whatever model the parent is using".
   */
  subagentModel: ModelSelection | null;
  /**
   * The model an image is shown to when `read_file` opens one.
   *
   * `null`: no vision model, so reading an image is a clear refusal rather than
   * a silent attempt at a model that cannot see.
   */
  visionModel: ModelSelection | null;
  /**
   * The model new conversations open on.
   *
   * The remembered (last-used) selection still wins: this is what a chat starts
   * on when nothing was remembered, not a lock. `null` keeps yesterday's
   * behaviour.
   */
  mainModel: ModelSelection | null;
  /**
   * Default reasoning effort for a new conversation's composer. `""` lets the
   * provider choose; the value is sent verbatim, like the composer's own.
   */
  mainReasoning: string;
  /**
   * The main model's context window, in tokens, overriding whatever the
   * provider or the GGUF header reports. `0` uses the detected value — and a
   * model with no detected window is one the auto-compression cannot read.
   */
  mainContextWindow: number;
  /** Tool rounds a main reply may spend before it is asked to answer in prose. */
  mainSteps: number;
  /** Tool rounds a sub-agent may spend, on the same budget mechanics. */
  subagentSteps: number;
  /**
   * Sub-agents allowed running at once. `0` removes the ceiling. A sixth
   * spawn at a full five is refused by name, so the model can schedule rather
   * than wonder where its agent went.
   */
  subagentParallel: number;
  /** Reasoning effort a sub-agent is sent. `""`: whatever its parent used. */
  subagentReasoning: string;
  /** Whether old context is compressed automatically before a reply. */
  autoCompact: boolean;
  /** When to compress: percent of the measured context window. */
  compactAtPct: number;
  /**
   * How much of the older end of a conversation the compress pass may rewrite,
   * as a fraction (0-1). The protected recent tail always wins over it.
   */
  compactReach: number;
  /**
   * What to aim at: after compressing, the request should fit in this fraction
   * of the window. Drives the pass depth, not a guarantee.
   */
  compactTarget: number;
  /** The last N messages that are always sent as they were. */
  compactProtect: number;
  /** Seconds a summarising model gets before the pass falls back to elision. */
  compactTimeout: number;
};

/** Applies defaults and sane bounds to one stored record. Clamping happens
 *  here, once, rather than in every page that shows a number. */
function withDefaults(saved: Partial<Preferences> | null | undefined): Preferences {
  const number = (value: unknown, fallback: number, min: number, max: number) => {
    const parsed = typeof value === "number" && Number.isFinite(value) ? value : fallback;
    return Math.min(max, Math.max(min, parsed));
  };
  // The legacy "compaction" field: "off" turns the new switch off, anything
  // else stays automatic. (The old fast/normal distinction is gone; depth is
  // the target now.)
  const autoCompact =
    typeof saved?.autoCompact === "boolean"
      ? saved.autoCompact
      : (saved as { compaction?: string } | undefined)?.compaction !== "off";
  const cleanSelection = (value: ModelSelection | null | undefined): ModelSelection | null =>
    value && typeof value === "object" ? value : null;
  return {
    sessionTitleModels: (saved?.sessionTitleModels ?? []).slice(0, MAX_TITLE_MODELS),
    messagingModels: (saved?.messagingModels ?? []).slice(0, MAX_MESSAGING_MODELS),
    subagentModel: cleanSelection(saved?.subagentModel),
    visionModel: cleanSelection(saved?.visionModel),
    mainModel: cleanSelection(saved?.mainModel),
    mainReasoning: typeof saved?.mainReasoning === "string" ? saved.mainReasoning : "",
    mainContextWindow: number(saved?.mainContextWindow, 0, 0, 10_000_000),
    mainSteps: number(saved?.mainSteps, DEFAULT_PREFERENCES.mainSteps, 1, 2000),
    subagentSteps: number(saved?.subagentSteps, DEFAULT_PREFERENCES.subagentSteps, 1, 2000),
    subagentParallel: number(saved?.subagentParallel, DEFAULT_PREFERENCES.subagentParallel, 0, 64),
    subagentReasoning: typeof saved?.subagentReasoning === "string" ? saved.subagentReasoning : "",
    autoCompact,
    compactAtPct: number(saved?.compactAtPct, DEFAULT_PREFERENCES.compactAtPct, 5, 99),
    compactReach: number(saved?.compactReach, DEFAULT_PREFERENCES.compactReach, 0, 1),
    compactTarget: number(saved?.compactTarget, DEFAULT_PREFERENCES.compactTarget, 0, 1),
    compactProtect: number(saved?.compactProtect, DEFAULT_PREFERENCES.compactProtect, 0, 1000),
    compactTimeout: number(saved?.compactTimeout, DEFAULT_PREFERENCES.compactTimeout, 5, 3600),
  };
}

/** The type as it travels to disk — the plain record, no helpers. */
type StoredPreferences = { [K in keyof Preferences]: Preferences[K] };

/**
 * Loads `app.preferences` once per enabled run and exposes a setter that
 * persists through `database_set_setting`. Shared so the chat view and the
 * settings modal agree on the key and shape.
 */
export function usePreferences(enabled: boolean) {
  const [preferences, setPreferences] = useState<Preferences>(() => withDefaults(null));

  useEffect(() => {
    if (!enabled) return;
    let mounted = true;
    invoke<Partial<Preferences> | null>("database_get_setting", { key: PREFERENCES_KEY })
      .then((saved) => {
        if (!mounted) return;
        // The original single-model shape is migrated withDefaults' legacy
        // compaction read, so an existing choice is kept rather than discarded.
        const legacy = (saved as { sessionTitleModel?: ModelSelection | null } | null)?.sessionTitleModel;
        setPreferences(withDefaults({
          ...(saved ?? {}),
          sessionTitleModels: saved?.sessionTitleModels ?? (legacy ? [legacy] : []),
        }));
      })
      .catch(() => undefined);
    return () => { mounted = false; };
  }, [enabled]);

  const savePreferences = useCallback((next: Preferences) => {
    setPreferences(next);
    // Stored whole, so nothing lands in the record that `usePreferences` did
    // not write: the backend reads the same keys it wrote.
    void invoke("database_set_setting", { key: PREFERENCES_KEY, value: next as StoredPreferences })
      .catch((reason: unknown) => {
        // Surface the failure but keep the optimistic local value so the
        // picker still reflects the user's last choice.
        console.error("Preferences could not be saved:", reason);
      });
  }, []);

  return { preferences, savePreferences };
}
