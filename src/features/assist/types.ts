/**
 * The assist feature's shapes, mirroring the Rust types.
 *
 * Two records rather than one: `AssistSettings` is what the user edits and what the backend
 * stores (clamped), and `AssistStatus` is what the backend has worked out *about* those
 * settings -- which model was chosen, whether it can be used right now, whether the socket is
 * up. Keeping them apart is what stops the page from having to derive "the local model is not
 * loaded" in TypeScript, where the answer would be a second opinion about the loader's state.
 *
 * There is nothing here for a *card*. An answer is drawn by the browser that asked for it, on
 * the page it belongs to; the app's window has no surface that shows one.
 */

import type { ModelSelection } from "../models/types";

/**
 * How hard a look is allowed to work, and what it may cost.
 *
 * A preset is a *bundle* of numbers, not a different pipeline: naming one writes the thresholds
 * below it, and `custom` is the record exactly as the user left it -- which is where the picker
 * moves the moment one of those numbers is edited. The name and the numbers travel together, so
 * the picker cannot show one level over another level's spend.
 */
export type AssistLevel = "efficient" | "normal" | "high" | "custom";

/** The kinds of help assist can offer, in the backend's wire spelling. */
export type AssistKind = "answer" | "explain" | "summarize" | "translate" | "check";

export type AssistSettings = {
  enabled: boolean;
  /** The answering model, in the same shape the other preferences use. */
  model: ModelSelection | null;
  /**
   * Whether the model may think before it answers.
   *
   * Off by default, and measured rather than guessed: on a local 2B, thinking spent the whole reply
   * budget and returned nothing twice over, while the same page answered in a fraction of the tokens
   * with it off. It is a setting because a large model is a different story — and it never disables
   * the retry, which is about replies that could not be read rather than about how they are made.
   */
  thinking: boolean;
  /** Free text the model is pointed at. A hint, never a filter. */
  hint: string;
  /** Subjects the user would rather be helped with, in order. A weight. */
  subjects: string[];
  level: AssistLevel;
  minChars: number;
  maxAskChars: number;
  maxPerHour: number;
  /** How many blocks one look considers at all. */
  maxCandidates: number;
  /** How many candidates one request carries. Ten at five per request is two requests. */
  candidatesPerRequest: number;
  /** Most cards one reply may carry. */
  maxAnswers: number;
  /** The reply's token ceiling, which a thinking model shares with its answer. */
  maxTokens: number;
  /** How many requests one look may make. */
  maxRequests: number;
  /**
   * The lowest confidence an answer may have and still be drawn.
   *
   * The model reports it already, but a small model's own judgement about when to stay quiet
   * is the least reliable thing it does — so the floor is enforced by the app rather than
   * requested by the prompt. Editing it moves the record to `custom`.
   */
  minConfidence: number;
  /**
   * The kinds of help this record may offer.
   *
   * Carried on the record (rather than re-derived here) so a record that moves to `custom`
   * keeps the breadth it had on its preset: an edit to one number must not silently disable
   * summaries or the two-pass look.
   */
  kinds: AssistKind[];
  /** Whether a look runs the locate/detail pipeline. Carried for the same reason as `kinds`. */
  locate: boolean;
  /** Run everything and draw nothing. */
  dryRun: boolean;
};

/**
 * What the chosen level is made of, including the bounds that are not settings.
 *
 * Read from the backend rather than re-derived here: `maxCandidates` and `maxAnswers` exist only
 * inside the level, and a second copy of them in TypeScript would be a description of the level
 * that could drift from the level itself.
 */
export type AssistLevelBounds = {
  maxCandidates: number;
  /** How many candidates one request carries -- what decides how many requests a look costs. */
  candidatesPerRequest: number;
  maxAnswers: number;
  maxAskChars: number;
  maxPerHour: number;
  /**
   * The reply's own ceiling — and, for a model that thinks out loud, the budget for its thinking
   * *and* its answer. It is the number to look at when a reply stops half-way.
   */
  maxTokens: number;
  /**
   * How many requests one look may make. The answer cap is per reply, so this is what decides how
   * much of a long short list is ever asked about.
   */
  maxRequests: number;
  /** Whether blocks the shape check could not name are offered at all. */
  unshaped: boolean;
  /** Whether the short system prompt is used. */
  briefPrompt: boolean;
  /** The kinds of help this level may offer: answer, explain, summarize, translate, check. */
  kinds: string[];
  /** Whether a look runs the locate/detail pipeline instead of one request. */
  locate: boolean;
  /** The confidence floor this level enforces. */
  minConfidence: number;
};

export type AssistBridge = {
  running: boolean;
  port: number | null;
  error: string | null;
  /** The profile labels of the browsers currently paired. */
  clients: string[];
};

/**
 * The settings record and its derived bounds, without live state.
 *
 * The page's fast path: it can draw every control from this answer while the
 * full status is still naming a local model or reading the bridge.
 */
export type AssistSettingsView = {
  settings: AssistSettings;
  levelBounds: AssistLevelBounds;
  suggestedSubjects: string[];
};

export type AssistStatus = {
  enabled: boolean;
  dryRun: boolean;
  settings: AssistSettings;
  /** The chosen model, named for a reader. */
  model: string | null;
  /** Whether it can be used now, in the app's own words. */
  modelReady: string | null;
  suggestedSubjects: string[];
  bridge: AssistBridge;
  /** How many requests were sent in the last hour, against the limit. */
  callsPastHour: number;
  levelBounds: AssistLevelBounds;
};

/**
 * The events the backend pushes at the app's own window.
 *
 * Only the switch reaches the app's UI: an outcome is drawn by the browser that asked for it,
 * on the page it belongs to, and the app has no surface that shows a card.
 */
export type AssistEvent = { kind: "state"; enabled: boolean };
