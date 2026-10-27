/** Mirrors `audio::voice::Summary`. */
export type VoiceSummary = {
  name: string;
  clips: number;
};

/** Mirrors `audio::voice::Status`. */
export type VoiceStatus = {
  /** The optional sherpa-onnx engine is on disk. */
  installed: boolean;
  /** On, with at least one voice enrolled. */
  enabled: boolean;
  /** Cosine a clip must reach to count as a person. */
  threshold: number;
  voices: VoiceSummary[];
  /** Takes held for the voice being created. */
  pending: number;
};
