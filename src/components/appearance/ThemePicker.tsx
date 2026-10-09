import type { CSSProperties } from "react";
import {
  THEMES,
  accentColor,
  inkFor,
  resolveTheme,
  type Appearance,
  type ThemeChoice,
  type ThemeId,
} from "../../features/appearance/appearance";

/**
 * The theme chooser: one tile per palette, each a miniature of the window.
 *
 * **A miniature, not a swatch.** Choosing between eight themes from a colour
 * chip asks the user to imagine the result; the tile shows it -- rail, header,
 * a user turn, two lines of reply, and the composer with its accent button. At
 * this size that is the whole of what the app looks like, which is exactly the
 * decision being made.
 *
 * **Drawn with the real tokens.** `data-theme` on the sample makes the same
 * rules that style the app style this box, so a palette cannot be added without
 * its preview being correct and a preview cannot drift from the thing it
 * previews. Nothing here is hand-coloured.
 *
 * **System previews what it resolves to.** It is a choice about *when* to change
 * rather than a palette, so its tile shows the palette the operating system is
 * currently asking for.
 *
 * The selection ring deliberately sits on a wrapper *outside* the `data-theme`
 * element: inside it, `--text` is the preview's own text colour, so a ring in
 * white on a dark tile would vanish against a light settings page.
 */

type ThemePickerProps = {
  appearance: Appearance;
  onChange: (appearance: Appearance) => void;
};

export default function ThemePicker({ appearance, onChange }: ThemePickerProps) {
  // Resolved once per render rather than per tile. Close enough for a preview:
  // a tile only has to be right when it is looked at, and the picker re-renders
  // whenever anything about the appearance changes.
  const systemTheme = resolveTheme("system");

  return (
    <div className="flex flex-wrap gap-x-3 gap-y-4">
      <Tile id="system" label="System" preview={systemTheme} appearance={appearance} onSelect={onChange} />
      {THEMES.map((theme) => (
        <Tile key={theme.id} id={theme.id} label={theme.label} preview={theme.id} appearance={appearance} onSelect={onChange} />
      ))}
    </div>
  );
}

function Tile({
  id,
  label,
  preview,
  appearance,
  onSelect,
}: {
  /** What the tile selects: a palette id, or `system`. */
  id: ThemeChoice;
  label: string;
  /** The palette to draw the sample with: `system` resolved to a real theme. */
  preview: ThemeId;
  appearance: Appearance;
  onSelect: (appearance: Appearance) => void;
}) {
  const selected = appearance.theme === id;
  // The user's own accent, so the sample shows the app they would actually get
  // rather than each palette's stock colour.
  const accent = accentColor(appearance, preview);

  return (
    <button
      type="button"
      onClick={() => onSelect({ ...appearance, theme: id })}
      aria-pressed={selected}
      aria-label={`${label} theme`}
      className="group flex flex-col items-start gap-1.5 rounded-md focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
    >
      <span className={`block rounded-md ${selected ? "ring-2 ring-[var(--text)]" : "ring-1 ring-[var(--line)] group-hover:ring-[var(--line-strong)]"}`}>
        <span
          data-theme={preview}
          style={{ "--accent": accent, "--accent-ink": inkFor(accent) } as CSSProperties}
          className="relative block h-[68px] w-[106px] overflow-hidden rounded-md bg-[var(--page)]"
        >
          {/* The rail: wordmark, the active view, and a few quieter rows. */}
          <span aria-hidden="true" className="absolute inset-y-0 left-0 w-[26px] border-r border-[var(--line)] bg-[var(--rail)]">
            <span className="absolute left-[6px] top-[7px] h-[3px] w-[15px] rounded-full bg-[var(--text)] opacity-80" />
            <span className="absolute left-[4px] top-[19px] h-[9px] w-[18px] rounded-[3px] bg-[color-mix(in_srgb,var(--accent)_20%,transparent)]" />
            <span className="absolute left-[8px] top-[22px] size-[3px] rounded-full bg-[var(--accent)]" />
            <span className="absolute left-[14px] top-[22px] h-[3px] w-[6px] rounded-full bg-[var(--accent)] opacity-80" />
            <span className="absolute left-[8px] top-[33px] h-[3px] w-[12px] rounded-full bg-[var(--quiet)] opacity-70" />
            <span className="absolute left-[8px] top-[40px] h-[3px] w-[14px] rounded-full bg-[var(--quiet)] opacity-50" />
            <span className="absolute left-[8px] top-[47px] h-[3px] w-[10px] rounded-full bg-[var(--quiet)] opacity-40" />
          </span>

          {/* The header: the conversation's name, and the model control at the
              far end where the real one sits. */}
          <span aria-hidden="true" className="absolute left-[26px] right-0 top-0 h-[13px] border-b border-[var(--line)]">
            <span className="absolute left-[6px] top-[5px] h-[3px] w-[30px] rounded-full bg-[var(--text)] opacity-70" />
            <span className="absolute right-[5px] top-[3px] h-[7px] w-[14px] rounded-[3px] border border-[var(--line)] bg-[var(--panel)]" />
          </span>

          {/* A user turn, right-aligned as it is in the transcript. */}
          <span aria-hidden="true" className="absolute right-[6px] top-[20px] h-[10px] w-[36px] rounded-[4px] bg-[var(--raised)]" />

          {/* Two lines of a reply. */}
          <span aria-hidden="true" className="absolute left-[32px] top-[35px] h-[3px] w-[58px] rounded-full bg-[var(--muted)] opacity-70" />
          <span aria-hidden="true" className="absolute left-[32px] top-[42px] h-[3px] w-[44px] rounded-full bg-[var(--muted)] opacity-45" />

          {/* The composer, with the send button carrying the accent -- the one
              place the accent is a solid fill, and so the one worth previewing. */}
          <span aria-hidden="true" className="absolute bottom-[5px] left-[31px] right-[6px] h-[12px] rounded-[4px] border border-[var(--line)] bg-[var(--panel)]">
            <span className="absolute left-[3px] top-[4px] h-[3px] w-[26px] rounded-full bg-[var(--quiet)] opacity-60" />
            <span className="absolute right-[2px] top-[2px] size-[7px] rounded-full bg-[var(--accent)]" />
          </span>
        </span>
      </span>
      <span className={`text-[12px] ${selected ? "text-[var(--text)]" : "text-[var(--muted)]"}`}>{label}</span>
    </button>
  );
}
