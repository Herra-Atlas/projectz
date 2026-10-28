import type { NotificationSound } from "./settings";

/**
 * The app's notification tones.
 *
 * **Synthesised, not shipped.** Six short recipes of sine and triangle notes are
 * a table and an oscillator; wav files would be assets to bundle, license and
 * decode for less control. Nothing here is a system notification -- the app makes
 * its own sound, so a tone plays whether or not Windows would have shown a toast.
 *
 * One `AudioContext` for the session, created on first use: creating one per
 * alert is a hardware handle per reply, and the preview and every event share it.
 * It is resumed on each play because a webview starts the context suspended until
 * the page has been interacted with.
 */

let context: AudioContext | null = null;
/**
 * Everything the tones pass through before the speakers.
 *
 * **The recipe gains are relative, not absolute.** A WebAudio gain of `1` is
 * full scale and `0.05` is about −26 dB -- inaudible on a laptop unless the user
 * happens to be at full volume, which is exactly the complaint that produced this
 * node. A master of 6 puts the loudest recipe near −8 dB: audible across a room
 * without clipping, because the layered tones are different frequencies rather
 * than one summed waveform.
 */
const MASTER_GAIN = 6;
let master: GainNode | null = null;

/** One note in a recipe, in seconds from the recipe's start. */
type Note = {
  frequency: number;
  delay: number;
  duration: number;
  gain: number;
  type?: OscillatorType;
};

/**
 * The sounds, as data.
 *
 * A recipe is the whole definition of a tone: adding a seventh means one entry
 * here and one name in `NOTIFICATION_SOUNDS`. Gains stay low because a chime
 * played over the user's own audio is worse than silence.
 */
const RECIPES: Record<NotificationSound, Note[]> = {
  // Two rising notes, the interval a doorbell uses: it reads as "done".
  chime: [
    { frequency: 880, delay: 0, duration: 0.16, gain: 0.05 },
    { frequency: 1174.66, delay: 0.14, duration: 0.22, gain: 0.045 },
  ],
  // One lower note for anyone who finds a doorbell shrill.
  soft: [{ frequency: 523.25, delay: 0, duration: 0.28, gain: 0.05 }],
  // A short upward blip: the least formal of the six.
  pop: [
    { frequency: 660, delay: 0, duration: 0.07, gain: 0.07 },
    { frequency: 990, delay: 0.05, duration: 0.09, gain: 0.05 },
  ],
  // A struck bell: a triangle fundamental with a quiet octave above it.
  bell: [
    { frequency: 1046.5, delay: 0, duration: 0.5, gain: 0.045, type: "triangle" },
    { frequency: 1568, delay: 0, duration: 0.34, gain: 0.02 },
  ],
  // Two wooden knocks, lower and drier than the rest.
  wood: [
    { frequency: 220, delay: 0, duration: 0.09, gain: 0.07, type: "triangle" },
    { frequency: 330, delay: 0.05, duration: 0.12, gain: 0.04, type: "triangle" },
  ],
  // Two even beats: an alert that does not sound like a success or a failure.
  pulse: [
    { frequency: 440, delay: 0, duration: 0.1, gain: 0.05 },
    { frequency: 440, delay: 0.16, duration: 0.1, gain: 0.05 },
  ],
};

/** A single decaying note. */
function playNote(ctx: AudioContext, output: AudioNode, start: number, note: Note) {
  const oscillator = ctx.createOscillator();
  const envelope = ctx.createGain();
  oscillator.type = note.type ?? "sine";
  oscillator.frequency.value = note.frequency;
  // A short attack and an exponential decay: a square-edged blip clicks, and a
  // chime that clicks reads as a fault rather than as a sound.
  const at = start + note.delay;
  envelope.gain.setValueAtTime(0.0001, at);
  envelope.gain.exponentialRampToValueAtTime(note.gain, at + 0.02);
  envelope.gain.exponentialRampToValueAtTime(0.0001, at + note.duration);
  oscillator.connect(envelope).connect(output);
  oscillator.start(at);
  oscillator.stop(at + note.duration + 0.02);
}

/** Plays one alert sound. */
export function playAlertSound(sound: NotificationSound): void {
  const recipe = RECIPES[sound];
  if (!recipe) return;
  const ctx = (context ??= new AudioContext());
  if (!master) {
    master = ctx.createGain();
    master.gain.value = MASTER_GAIN;
    master.connect(ctx.destination);
  }
  void ctx.resume().catch(() => undefined);
  const now = ctx.currentTime;
  for (const note of recipe) playNote(ctx, master, now, note);
}
