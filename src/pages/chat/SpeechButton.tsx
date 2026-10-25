import { LoaderCircle, Volume2 } from "lucide-react";
import type { SpeechPhase } from "../../features/tts/useSpeech";

/**
 * Reads one message aloud.
 *
 * Three states, because they are three different facts:
 *
 * - idle: a speaker and "Read".
 * - `synthesizing`: a spinner and "Preparing". The voice is being made; there is
 *   nothing to stop yet, and the press cancels the wait.
 * - `playing`: the equalizer and "Stop". Sound is coming out right now, which is
 *   what the animation is for and what the press ends.
 *
 * The label is what the press will do; the mark says which state it is in. They
 * are deliberately not the same word: "Stop" on a spinner claims there is audio
 * to stop.
 */
export function SpeechButton({
  active,
  phase,
  disabled,
  onPress,
}: {
  active: boolean;
  /** Null unless this message is the one being read. */
  phase: SpeechPhase | null;
  disabled: boolean;
  onPress: () => void;
}) {
  const playing = active && phase === "playing";
  const preparing = active && phase === "synthesizing";
  return (
    <button
      type="button"
      onClick={onPress}
      disabled={disabled}
      aria-pressed={active}
      aria-label={active ? "Stop reading" : "Read aloud"}
      title={active ? "Stop reading" : "Read aloud"}
      className={`inline-flex min-h-8 items-center gap-1 rounded px-1.5 hover:bg-[var(--raised)] hover:text-[var(--text)] disabled:opacity-40 ${active ? "text-[var(--accent)]" : ""}`}
    >
      {playing ? (
        <span className="pz-eq inline-flex h-3 items-end gap-[2px]" aria-hidden="true">
          <span className="h-1.5 w-[2px] rounded-sm bg-current" />
          <span className="h-3 w-[2px] rounded-sm bg-current" />
          <span className="h-2 w-[2px] rounded-sm bg-current" />
        </span>
      ) : preparing ? (
        <LoaderCircle size={13} className="animate-spin" />
      ) : (
        <Volume2 size={13} />
      )}
      <span>{playing ? "Stop" : preparing ? "Preparing" : "Read"}</span>
    </button>
  );
}
