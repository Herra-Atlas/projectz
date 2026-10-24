/**
 * What the backend knows about the installed voice.
 *
 * Mirrors `VoiceInfo` in `src-tauri/src/tts/voice.rs`. `sampleRate` is the rate of
 * the PCM `tts_speak` returns -- the player needs it, and a wrong value does not
 * fail, it just changes the pitch.
 */
export type VoiceInfo = {
  available: boolean;
  name: string;
  sampleRate: number;
  /** Why it cannot be used, when it cannot. */
  reason: string | null;
};

/** One voice the app can speak with. Mirrors `VoiceEntry`. */
export type VoiceEntry = {
  /** The model file's path, which is also what a selection stores. */
  id: string;
  name: string;
  sampleRate: number;
  size: number;
  /** `piper` or `audiocpp` — the engine a path resolves to, told by its folder. */
  engine: string;
};

/** The stored speech settings, with the bounds the page draws against. */
export type TtsSettings = {
  /** The chosen voice's path, or null for the built-in one. */
  voice: string | null;
  /** 1 is the voice's own pace. */
  speed: number;
  minSpeed: number;
  maxSpeed: number;
  /** Whether code is left out of what is spoken. Applies to every voice. */
  skipCode: boolean;
  /** Whether markdown markup is left out of what is spoken. Applies to every voice. */
  skipMarkdown: boolean;
  /** Whether replies are read as they finish, without being pressed. */
  autoSpeak: boolean;
  /** Whether the auto read holds off on a voice that would need a cold start. */
  autoSpeakOnlyLoaded: boolean;
};

/** One voice's pace, with the bounds its own page draws against. */
export type VoiceSpeed = {
  speed: number;
  minSpeed: number;
  maxSpeed: number;
};
