/** One installed wake model. Mirrors `wake::ModelRow`. */
export type WakeModel = {
  /** Absolute path; the id the page selects by. */
  path: string;
  name: string;
  /** The family id for an audio.cpp pack, `openWakeWord` for an ONNX head. */
  family: string;
  /** The engine that runs it: `audio.cpp` or `openWakeWord`. */
  engine: string;
  size: number;
  /** Whether its family ships a spec, which is what the edit page draws. */
  editable: boolean;
};

/** Mirrors `wake::WakeStatus`. */
export type WakeStatus = {
  enabled: boolean;
  /** The microphone is open right now. */
  running: boolean;
  /** The model in force, or null when nothing is installed. */
  model: string | null;
  threshold: number;
  models: WakeModel[];
  audioEngineInstalled: boolean;
  audioEngineVersion: string | null;
  openWakeWordInstalled: boolean;
  openWakeWordVersion: string | null;
};

/** What `wake-detected` carries. */
export type WakeDetection = {
  text: string;
  confidence: number;
};
