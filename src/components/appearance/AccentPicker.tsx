import { useEffect, useRef, useState } from "react";
import {
  ACCENT_PRESETS,
  accentColor,
  inkFor,
  isHexColor,
  themeKind,
  type Appearance,
  type ThemeId,
} from "../../features/appearance/appearance";

/**
 * The accent swatches and the custom colour field.
 *
 * **Split out because the custom field needs machinery nothing else on the page
 * does.** A native colour input fires `input` continuously while its picker is
 * being dragged, and each event used to go straight to the app's appearance
 * state -- which lives on `AppShell`, so every pointer move re-rendered the
 * sidebar, the transcript and both panels, and fired a database write. Dragging
 * the wheel lagged because a colour picker was repainting the entire window
 * dozens of times a second.
 *
 * A drag now does two cheap things and one rare one:
 *
 * - the local draft updates, which re-renders this component and nothing else;
 * - the accent is written straight onto the document element, so every screen
 *   recolours live -- they all read the custom properties, and changing one is
 *   not a React render;
 * - the real commit is debounced until the wheel has been still, so the app-wide
 *   state change and the settings write happen once per gesture.
 *
 * The pending value is flushed on unmount, so closing the dialog mid-drag still
 * saves the colour the user was looking at.
 */
type AccentPickerProps = {
  appearance: Appearance;
  /** The theme in force, so a swatch previews the colour actually on screen. */
  theme: ThemeId;
  onChange: (appearance: Appearance) => void;
};

/** How long the wheel must be still before the choice is written down. */
const COMMIT_DELAY = 150;

export default function AccentPicker({ appearance, theme, onChange }: AccentPickerProps) {
  /** The colour being dragged, held here so the input stays responsive. */
  const [draft, setDraft] = useState<string | null>(null);
  const pending = useRef<string | null>(null);
  const timer = useRef<number | null>(null);
  /**
   * Read through a ref inside the timer and the unmount cleanup, both of which
   * outlive the render that created them -- a timeout capturing its render's
   * `appearance` would write a value one step out of date.
   */
  const latest = useRef({ appearance, onChange });
  latest.current = { appearance, onChange };

  /** Writes whatever the drag has settled on, and cancels the pending timer. */
  const flush = () => {
    if (timer.current !== null) {
      window.clearTimeout(timer.current);
      timer.current = null;
    }
    const value = pending.current;
    pending.current = null;
    if (value) latest.current.onChange({ ...latest.current.appearance, accent: "custom", customAccent: value });
  };

  useEffect(() => () => flush(), []);

  /** A preset is one click, so it commits immediately and drops any drag. */
  const choosePreset = (id: string) => {
    if (timer.current !== null) {
      window.clearTimeout(timer.current);
      timer.current = null;
    }
    pending.current = null;
    setDraft(null);
    onChange({ ...appearance, accent: id });
  };

  const onCustom = (value: string) => {
    if (!isHexColor(value)) return;
    setDraft(value);
    pending.current = value;
    // Preview without touching app state. Changing a custom property is not a
    // render, so nothing in React's tree runs -- the browser repaints the
    // colours and that is the whole cost.
    const root = document.documentElement;
    root.style.setProperty("--accent", value);
    root.style.setProperty("--accent-ink", inkFor(value));
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(flush, COMMIT_DELAY);
  };

  return (
    <div className="flex flex-wrap items-center gap-2">
      {ACCENT_PRESETS.map((preset) => {
        const selected = appearance.accent === preset.id;
        return (
          <button
            key={preset.id}
            type="button"
            onClick={() => choosePreset(preset.id)}
            aria-label={`${preset.label} accent`}
            aria-pressed={selected}
            title={preset.label}
            className={`grid size-8 place-items-center rounded-md border transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)] ${selected ? "border-[var(--text)]" : "border-[var(--line)] hover:border-[var(--line-strong)]"}`}
          >
            {/* The colour for the theme in force, not for both, so the row
                previews what is actually on screen. Presets carry a value per
                *kind* of theme, so the theme is resolved to its kind first --
                indexing by theme id would be undefined for every palette past
                the first two. */}
            <span className="size-4 rounded-full" style={{ backgroundColor: preset[themeKind(theme)] }} />
          </button>
        );
      })}
      <label
        className={`flex h-8 cursor-pointer items-center gap-2 rounded-md border px-2 text-[12px] transition-colors ${appearance.accent === "custom" ? "border-[var(--text)] text-[var(--text)]" : "border-[var(--line)] text-[var(--muted)] hover:border-[var(--line-strong)]"}`}
      >
        <input
          type="color"
          value={draft ?? appearance.customAccent ?? accentColor(appearance, theme)}
          onChange={(event) => onCustom(event.target.value)}
          // Also the commit path for a picker that closes without a final
          // `input`, which is how the operating system's dialog reports a
          // cancelled or accepted drag.
          onBlur={flush}
          aria-label="Custom accent colour"
          className="size-5 cursor-pointer rounded border-0 bg-transparent p-0"
        />
        Custom
      </label>
    </div>
  );
}
