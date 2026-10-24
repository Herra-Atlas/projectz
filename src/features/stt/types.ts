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
  /** True when the file belongs to the audio.cpp engine, under `models/audio`. */
  audio: boolean;
  /** The audio.cpp family the model names, empty for a whisper model. */
  family: string;
};

/** Mirrors `stt::model::SttSettings`. */
export type SttSettings = {
  /** The chosen model's path, or null for the first one found. */
  model: string | null;
};
