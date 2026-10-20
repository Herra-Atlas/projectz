import type { ModelSelection } from "../models/types";

/**
 * The fixed headings the backend stores.
 *
 * The names are the wire names, not display labels: a section is addressed by the
 * model in an operation, so renaming a label here must not change what is stored.
 * Labels live in `tasteFormat.ts`.
 */
export type TasteSection =
  | "identity"
  | "work"
  | "preferences"
  | "communication"
  | "interests"
  | "dislikes"
  | "current";

/** The stored configuration, mirroring `taste::settings::TasteSettings`. */
export type TasteSettings = {
  enabled: boolean;
  /** Ordered fallback chain. The first model that answers wins. */
  models: ModelSelection[];
  intervalDays: number;
  nextRunAt?: string | null;
};

/** One learned fact. */
export type TasteItem = {
  id: number;
  section: TasteSection;
  text: string;
  confidence: number;
  lastSeenAt: string;
  updatedAt: string;
};

/** One learning run. */
export type TasteRun = {
  id: number;
  startedAt: string;
  finishedAt?: string | null;
  status: "completed" | "failed" | "skipped" | string;
  reason?: string | null;
  modelId?: string | null;
  providerId?: string | null;
  sessionsRead: number;
  messagesRead: number;
  promptTokens?: number | null;
  completionTokens?: number | null;
  added: number;
  updated: number;
  removed: number;
  report?: string | null;
  /** The profile as it stood when the run finished, for debugging. */
  document?: string | null;
};

/** Everything `taste_status` returns. */
export type TasteStatus = {
  enabled: boolean;
  settings: TasteSettings;
  items: TasteItem[];
  /** The profile as the chat receives it. Empty when nothing was learned. */
  document: string;
  lastRun?: TasteRun | null;
  running: boolean;
  nextRunAt?: string | null;
};

/** The event the backend emits on a state change or a run boundary. */
export type TasteEvent = {
  kind: "state" | "started" | "completed" | "failed" | "skipped" | string;
  run?: TasteRun | null;
};
