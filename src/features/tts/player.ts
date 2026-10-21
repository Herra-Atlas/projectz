/**
 * The audio side of reading aloud.
 *
 * # Why one context for the window
 *
 * An `AudioContext` is a handle on an output device, and a browser only allows a
 * small number of them. Creating one per message would leak one per message, so
 * it is made on the first press and reused. Making it lazily also matters for
 * start-up: the app should not open an audio device for a feature nobody used.
 *
 * # Why the samples are widened by hand
 *
 * Piper writes 16-bit little-endian mono and nothing else -- no container, no
 * decode library. A `DataView`-free `Int16Array` over the same bytes is that
 * conversion, and every platform this ships on is little-endian.
 */

let context: AudioContext | null = null;
let playing: AudioBufferSourceNode | null = null;

function audio(): AudioContext {
  context ??= new AudioContext();
  return context;
}

/** Stops whatever is playing. Safe to call when nothing is. */
export function stopPlayback(): void {
  const node = playing;
  if (!node) return;
  // Cleared before `stop()` so the `onended` handler it triggers cannot mistake
  // itself for a later source and clear the wrong one.
  playing = null;
  node.stop();
}

/**
 * Plays `bytes` as 16-bit mono PCM at `sampleRate`.
 *
 * Anything already playing is stopped first: two replies read at once is not a
 * state the button can produce, and the second press should replace the first
 * rather than talk over it.
 */
export function playPcm(bytes: ArrayBuffer, sampleRate: number, onEnded: () => void): void {
  stopPlayback();

  const mono = new Int16Array(bytes);
  const audioContext = audio();
  // A context created before any gesture can start suspended; a press is a
  // gesture, so this is the moment it is allowed to resume.
  if (audioContext.state === "suspended") void audioContext.resume();

  const buffer = audioContext.createBuffer(1, mono.length, sampleRate);
  const channel = buffer.getChannelData(0);
  for (let index = 0; index < mono.length; index += 1) {
    channel[index] = mono[index] / 32768;
  }

  const node = audioContext.createBufferSource();
  node.buffer = buffer;
  node.connect(audioContext.destination);
  node.onended = () => {
    // Only the current source reports back; a superseded one has already had
    // `playing` cleared and must not reset the newest message's state.
    if (playing !== node) return;
    playing = null;
    onEnded();
  };
  playing = node;
  node.start();
}
