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
/**
 * The source on the output device, and the callback its end owes.
 *
 * The callback is kept beside the node rather than only on `onended`, because a
 * stop has to settle the same promise a natural end does -- otherwise the
 * sentence queue in `useSpeech` would keep an await open forever after Stop.
 */
let playing: { node: AudioBufferSourceNode; ended: () => void } | null = null;

function audio(): AudioContext {
  context ??= new AudioContext();
  return context;
}

/**
 * Opens the output device during a user gesture.
 *
 * A browser will not let an `AudioContext` created outside a gesture make sound;
 * it stays `suspended`, and a reply that is read automatically arrives with no
 * gesture anywhere near it. Called once from the chat page on the first press or
 * key, so by the time a reply finishes there is a running context to play into
 * rather than a silent one. Safe to call any number of times.
 */
export function unlockAudio(): void {
  const audioContext = audio();
  if (audioContext.state === "suspended") void audioContext.resume();
}

/** Stops whatever is playing. Safe to call when nothing is. */
export function stopPlayback(): void {
  const current = playing;
  if (!current) return;
  // Cleared before `stop()` so the `onended` handler it triggers cannot settle
  // the same promise a second time.
  playing = null;
  current.node.stop();
  current.ended();
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
  const buffer = audioContext.createBuffer(1, mono.length, sampleRate);
  const channel = buffer.getChannelData(0);
  for (let index = 0; index < mono.length; index += 1) {
    channel[index] = mono[index] / 32768;
  }

  const node = audioContext.createBufferSource();
  node.buffer = buffer;
  node.connect(audioContext.destination);
  // Resolved exactly once, whether the sound finishes or is stopped.
  let settled = false;
  const ended = () => {
    if (settled) return;
    settled = true;
    onEnded();
  };
  node.onended = () => {
    // Only the current source reports back; a superseded one has already had
    // `playing` cleared and must not reset the newest message's state.
    if (playing?.node !== node) return;
    playing = null;
    ended();
  };
  playing = { node, ended };
  // A context that is still suspended cannot make sound and will never fire
  // `onended`, which used to leave the Read button spinning on a reply nobody
  // heard. Waiting for `resume()` before starting is what makes an automatic
  // read audible when the context was opened before the gesture that allows it.
  const start = () => node.start();
  if (audioContext.state === "suspended") {
    void audioContext.resume().then(start).catch(() => start());
  } else {
    start();
  }
}
