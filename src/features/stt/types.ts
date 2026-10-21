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
