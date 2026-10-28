import { Play } from "lucide-react";
import Select from "../../Select";
import { playAlertSound } from "../../../features/notifications/sound";
import {
  NOTIFICATION_SOUNDS,
  SOUND_LABELS,
  type NotificationSound,
} from "../../../features/notifications/settings";

/**
 * The sound for one alert: a list to choose from and a button to hear it again.
 *
 * **Choosing plays.** A picker with six names and no way to hear them makes the
 * user choose blind -- the tones differ in character, which a word cannot carry --
 * so a change plays immediately. The button is for the second listen, and for
 * comparing two without changing the setting back and forth.
 */
export function SoundPicker({
  value,
  onChange,
  label,
}: {
  value: NotificationSound;
  onChange: (next: NotificationSound) => void;
  label: string;
}) {
  return (
    <div className="flex items-center gap-1.5">
      <Select
        label={label}
        value={value}
        size="sm"
        options={NOTIFICATION_SOUNDS.map((sound) => ({ value: sound, label: SOUND_LABELS[sound] }))}
        onChange={(next) => {
          const sound = next as NotificationSound;
          onChange(sound);
          playAlertSound(sound);
        }}
        className="w-28"
      />
      <button
        type="button"
        onClick={() => playAlertSound(value)}
        aria-label={`Play ${SOUND_LABELS[value]}`}
        title="Play"
        className="grid size-8 place-items-center rounded-md border border-[var(--line)] text-[var(--muted)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
      >
        <Play size={13} />
      </button>
    </div>
  );
}
