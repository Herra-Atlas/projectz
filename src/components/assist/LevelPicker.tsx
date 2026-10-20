import { Check } from "lucide-react";
import type { AssistLevel, AssistLevelBounds } from "../../features/assist/types";

/**
 * The presets, cheapest first, each with the one thing it adds over the level before it.
 *
 * The line is what the *spend* cannot say: `candidates / answers / tokens` describe how
 * much a level buys, not why a reader would pick it, and a card whose only text is a
 * number is a card that gets clicked for the wrong reason. `custom` is not a card --
 * it is where the record goes when one of the numbers behind a preset is edited, and a
 * fourth card meaning "keep what you have" would be a button that does nothing.
 */
const LEVELS = [
  { id: "efficient", label: "Efficient", hint: "Answers questions, and nothing else." },
  { id: "normal", label: "Normal", hint: "Adds explanations, checks and translations." },
  { id: "high", label: "High", hint: "Adds summaries, and reads the page in two passes." },
] as const satisfies ReadonlyArray<{ id: AssistLevel; label: string; hint: string }>;

/**
 * What a level is made of, in one line.
 *
 * Every number comes from the backend's own accessors rather than from a copy here: the
 * level exposes them through `levelBounds`, and a description that repeated them would be
 * a description that could drift from what is running. Singular/plural is handled because
 * "1 candidates" is how a card stops being trusted.
 */
function spend(bounds: AssistLevelBounds) {
  const candidates =
    bounds.maxCandidates === 1 ? "1 candidate" : `${bounds.maxCandidates} candidates`;
  const perRequest =
    bounds.candidatesPerRequest === 1
      ? "1 per request"
      : `${bounds.candidatesPerRequest} per request`;
  const answers = bounds.maxAnswers === 1 ? "1 answer" : `${bounds.maxAnswers} answers`;
  const requests = bounds.maxRequests === 1 ? "1 request" : `${bounds.maxRequests} requests`;
  return `${candidates} (${perRequest}) · at most ${answers} per reply, ${bounds.maxTokens} tokens · at most ${requests} a look, ${bounds.maxPerHour} an hour`;
}

/**
 * How hard Assist works, drawn as three choices rather than a segmented control.
 *
 * A segmented control is right for a binary or a vocabulary of equals (Off / Normal / Fast).
 * These three differ in *what they do*, and the thing that tells them apart does not fit in
 * a button: the old row carried the whole explanation in its description and left the three
 * names looking interchangeable. Cards give each name a line of its own, and the spend for
 * the level in force sits underneath, so the numbers describe the choice instead of
 * competing with it.
 *
 * The row is a single-select group: pressing a card moves the record, and `aria-pressed`
 * says which one is in force. A record that is `custom` (a number was edited) leaves all
 * three unpressed, which is the truth -- none of the presets is what is running.
 */
export default function LevelPicker({
  level,
  bounds,
  onChoose,
}: {
  /** The level in force; `custom` when the numbers no longer match any preset. */
  level: AssistLevel;
  /** What that level is made of, read from the backend. */
  bounds: AssistLevelBounds;
  onChoose: (level: AssistLevel) => void;
}) {
  return (
    <div className="px-4 py-4">
      <div role="group" aria-label="Performance" className="grid grid-cols-3 gap-1.5">
        {LEVELS.map((preset) => {
          const active = level === preset.id;
          return (
            <button
              key={preset.id}
              type="button"
              aria-pressed={active}
              onClick={() => onChoose(preset.id)}
              className={`group relative flex min-h-[74px] flex-col justify-start rounded-lg border px-3 py-2.5 text-left transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)] ${
                active
                  ? "border-[color-mix(in_srgb,var(--accent)_50%,var(--line))] bg-[color-mix(in_srgb,var(--accent)_10%,transparent)]"
                  : "border-[var(--line)] hover:border-[var(--line-strong)] hover:bg-[var(--raised)]"
              }`}
            >
              <span className="flex items-center gap-1.5">
                <span
                  className={`text-[13px] font-medium ${active ? "text-[var(--accent)]" : "text-[var(--text)]"}`}
                >
                  {preset.label}
                </span>
                {active && <Check size={13} className="shrink-0 text-[var(--accent)]" />}
              </span>
              <span className="mt-1 text-[11px] leading-4 text-[var(--muted)]">{preset.hint}</span>
            </button>
          );
        })}
      </div>
      {/* The numbers for the level in force, outside the cards: they belong to the
          choice that was made, not to the choice being offered. */}
      <p className="mt-2.5 px-0.5 text-[11px] leading-4 tabular-nums text-[var(--quiet)]">
        {level === "custom" ? `Your own numbers · ${spend(bounds)}` : spend(bounds)}
      </p>
    </div>
  );
}
