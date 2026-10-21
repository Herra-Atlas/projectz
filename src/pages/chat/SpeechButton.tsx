import { Volume2 } from "lucide-react";

/**
 * Reads one message aloud.
 *
 * The icon stays the same in both states on purpose: a speaker that turns into a
 * stop square looks like a different control, and this one is a toggle whose
 * label already says what the press will do. The label is the state, the icon is
 * what the button is for.
 */
export function SpeechButton({
  active,
  disabled,
  onPress,
}: {
  active: boolean;
  disabled: boolean;
  onPress: () => void;
}) {
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
      <Volume2 size={13} />
      <span>{active ? "Stop" : "Read"}</span>
    </button>
  );
}
