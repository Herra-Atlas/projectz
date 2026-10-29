/** Mirrors `SttStatus` in `src-tauri/src/stt/commands.rs`. */
export type SttStatus = {
  /** The whisper.cpp build is unpacked and has its executable. */
  engineInstalled: boolean;
  /** The release the app installs, shown so a build is identifiable. */
  engineVersion: string;
  /** How many models are on disk. */
  models: number;
};

/** What `stt-progress` carries while the engine archive arrives. */
export type SttProgress = {
  received: number;
  total: number | null;
};

/** One `ggml-*.bin` model on disk. Mirrors `stt::models::Model`. */
export type SttModel = {
  /** Absolute path; both the id and what the engine is handed. */
  path: string;
  name: string;
  /** The repository folder it was installed under — the family, for audio models. */
  repo: string;
  size: number;
  /** Parakeet is a different model family, read by a different executable. */
  parakeet: boolean;
  /**
   * Which engine serves this file: `whisper`, `audiocpp` or `llamacpp`.
   *
   * A string rather than a flag per engine, so a fourth engine is one more value
   * instead of another boolean that can contradict the ones already there.
   */
  engine: string;
  /** The audio.cpp family the model names, empty for a whisper model. */
  family: string;
};

/** Mirrors `stt::models::SttSettings`. */
export type SttSettings = {
  /** The chosen model's path, or null for the first one found. */
  model: string | null;
  /**
   * The capture endpoint to record from, by name, or null for the system
   * default. A name rather than an index: the list renumbers as devices come
   * and go.
   */
  device: string | null;
  /** Keep the recognition model in memory between recordings. */
  keepLoaded: boolean;
  /**
   * Send a finished transcript as a message instead of leaving it in the draft.
   *
   * Read on each press rather than watched for changes, so a switch flipped in
   * Settings applies to the next recording without an event to keep in step.
   */
  autoSend: boolean;
  /** Quiet, in milliseconds, that ends a recording once speech has been heard. */
  silenceMs: number;
};

/** Mirrors `stt::commands::Inputs`. */
export type SttInputs = {
  /** Every capture endpoint this machine offers, sorted. */
  devices: string[];
  /** What Windows would use with nothing chosen, or null when it has no input. */
  default: string | null;
};
