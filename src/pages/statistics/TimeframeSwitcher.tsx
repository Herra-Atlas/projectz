import { TIMEFRAMES, type TimeframeKey } from "../../features/statistics/timeframes";

/** Timeframe control for the statistics header.
 *
 * Shaped like `Segmented` in `SettingsSection.tsx` so the two pages share a
 * control vocabulary, but it takes labels rather than raw values, which is what
 * the statistics page needs: the stored key is `quarter` while the label the
 * user reads is "3 months". */
export default function TimeframeSwitcher({
  value,
  onChange,
}: {
  value: TimeframeKey;
  onChange: (next: TimeframeKey) => void;
}) {
  return (
    <div role="group" aria-label="Statistics timeframe" className="inline-flex shrink-0 gap-0.5 rounded-lg bg-[var(--rail)] p-0.5">
      {TIMEFRAMES.map((entry) => {
        const selected = entry.key === value;
        return (
          <button
            key={entry.key}
            type="button"
            aria-pressed={selected}
            onClick={() => onChange(entry.key)}
            className={`min-h-8 rounded-md px-2.5 text-xs transition-colors focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)] ${selected ? "bg-[var(--raised)] text-[var(--text)]" : "text-[var(--muted)] hover:text-[var(--text)]"}`}
          >
            {entry.label}
          </button>
        );
      })}
    </div>
  );
}
