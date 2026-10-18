/**
 * The appearance settings: which theme the app renders in, and which colour it
 * accents with.
 *
 * Both are choices about the same thing -- the palette every screen is drawn
 * from -- so they live in one record and one settings page rather than being
 * split between "theme" and "colours".
 *
 * The palettes themselves are not here. `App.css` owns them, keyed by the same
 * ids this module uses; this decides *which* palette is in force and what the
 * accent resolves to, and the two meet only through the attributes and custom
 * properties on the document element.
 */

/**
 * Which half of the palette scale a theme belongs to.
 *
 * Only used to pick an accent: a colour legible as text on near-black is not
 * legible as text on white, so each accent preset carries a value for each half
 * and a theme says which half it is. A new theme therefore costs no new accent
 * values -- it declares which side it is on, beside its own id below.
 */
export type ThemeKind = "dark" | "light";

/**
 * The palettes the app can render in, in the order the picker shows them.
 *
 * **The one list.** A theme is declared here and in `App.css`, whose
 * `[data-theme=...]` block holds its colour values; nothing else enumerates
 * them. The id and the kind used to sit in two separate arrays, so a light theme
 * added to one and forgotten in the other silently drew dark accents on a light
 * ground. `ThemeId` below is derived from this list rather than restated beside
 * it, so a theme cannot be half-added.
 */
export const THEMES = [
  { id: "dark", label: "Dark", kind: "dark" },
  { id: "midnight", label: "Midnight", kind: "dark" },
  { id: "slate", label: "Slate", kind: "dark" },
  { id: "forest", label: "Forest", kind: "dark" },
  { id: "plum", label: "Plum", kind: "dark" },
  { id: "ember", label: "Ember", kind: "dark" },
  { id: "ocean", label: "Ocean", kind: "dark" },
  { id: "light", label: "Light", kind: "light" },
  { id: "paper", label: "Paper", kind: "light" },
  { id: "mist", label: "Mist", kind: "light" },
  { id: "sage", label: "Sage", kind: "light" },
  { id: "blush", label: "Blush", kind: "light" },
] as const satisfies readonly { id: string; label: string; kind: ThemeKind }[];

/**
 * A palette the app can render in. Each one has a matching `[data-theme=...]`
 * rule in `App.css`, so adding one means adding it here *and* there.
 */
export type ThemeId = (typeof THEMES)[number]["id"];

/** What the user chose. `system` follows the operating system. */
export type ThemeChoice = ThemeId | "system";

export type Appearance = {
  theme: ThemeChoice;
  /** Id of the chosen preset, or `"custom"` when `customAccent` is in use. */
  accent: string;
  /** The hex chosen with the colour picker. Read only when `accent` is `"custom"`. */
  customAccent: string | null;
};

/** Settings key holding the Appearance page values. */
export const APPEARANCE_KEY = "app.appearance";

/**
 * Where the record is mirrored for the pre-paint script in `index.html`.
 *
 * A second copy of the same fact, which is normally the thing to avoid -- but
 * the database cannot be read before the first paint, and without this a light
 * theme flashes dark on every launch. The mirror is written by the hook below
 * and is never read back as the source of truth.
 */
export const APPEARANCE_STORAGE_KEY = "projectz.appearance.v1";

/** Every value `theme` may hold, for validating what was stored. */
const THEME_CHOICES = new Set<string>([...THEMES.map((theme) => theme.id), "system"]);

/** Which half of the accent scale a theme draws from. */
export function themeKind(theme: ThemeId): ThemeKind {
  return THEMES.find((entry) => entry.id === theme)?.kind ?? "dark";
}

/**
 * The value for `<meta name="theme-color">` in a palette: the tint the operating
 * system paints the window frame with.
 *
 * Derived from the kind rather than read from `App.css`, because the pre-paint
 * script that needs it has no stylesheet to read. Stating it here is also what
 * lets `index.html` stop naming the light themes itself -- it kept its own list,
 * and had already missed `mist`.
 */
export function themeColor(theme: ThemeId): string {
  return themeKind(theme) === "light" ? "#fbfbfc" : "#0e0e10";
}

/**
 * What the native window frame is told to be.
 *
 * The OS draws the title bar and its buttons, not the webview, so unlike every
 * other surface in the app this one cannot read a CSS token -- it is set through
 * a Tauri command. It also only knows light and dark, never a palette: every
 * theme collapses to one of these three.
 */
export type WindowTheme = ThemeKind | "system";

/**
 * The frame value for a choice. `system` follows the OS, which is exactly what a
 * `system` theme means; a pinned theme becomes the half it belongs to.
 *
 * This is what stops Rust needing its own list of light themes. The value is
 * stored beside the appearance record, so the frame is painted correctly on the
 * next launch *before* the frontend runs -- see `useAppearance` and `window.rs`.
 */
export function windowTheme(choice: ThemeChoice): WindowTheme {
  return choice === "system" ? "system" : themeKind(choice);
}

export type AccentPreset = {
  id: string;
  label: string;
  /** The colour used by the dark-half themes. */
  dark: string;
  /**
   * The colour used by the light-half themes.
   *
   * Deeper than the dark value rather than the same hex. The accent is used as
   * text in several places -- the active nav item, the mode label, a focus ring
   * -- and a colour light enough to read as a fill on near-black is too pale to
   * read as text on white. The two are chosen apart, not derived from one value.
   */
  light: string;
};

/** The offered accents, in the order they are shown. */
export const ACCENT_PRESETS: AccentPreset[] = [
  { id: "amber", label: "Amber", dark: "#e2b57c", light: "#a9660f" },
  { id: "blue", label: "Blue", dark: "#7fb0e0", light: "#1f6fbf" },
  { id: "violet", label: "Violet", dark: "#b0a0e8", light: "#6d3fd1" },
  { id: "green", label: "Green", dark: "#8fc98a", light: "#2f7d3a" },
  { id: "rose", label: "Rose", dark: "#e59ab0", light: "#b83f63" },
  { id: "teal", label: "Teal", dark: "#6cc7bd", light: "#14766f" },
];

/**
 * What a fresh install renders.
 *
 * `dark` and the amber preset, which is exactly what the app looked like before
 * the setting existed -- so upgrading changes nothing until the user asks it to.
 */
export const DEFAULT_APPEARANCE: Appearance = { theme: "dark", accent: "amber", customAccent: null };

/** True for a six-digit hex colour, the only shape the colour input produces. */
export function isHexColor(value: unknown): value is string {
  return typeof value === "string" && /^#[0-9a-fA-F]{6}$/.test(value);
}

/** A stored or partial record, filled in and clamped to values we understand. */
export function normalizeAppearance(saved: Partial<Appearance> | null | undefined): Appearance {
  const accent = typeof saved?.accent === "string" && saved.accent ? saved.accent : DEFAULT_APPEARANCE.accent;
  const theme = typeof saved?.theme === "string" && THEME_CHOICES.has(saved.theme) ? saved.theme as ThemeChoice : "dark";
  return {
    theme,
    accent,
    customAccent: isHexColor(saved?.customAccent) ? saved.customAccent : null,
  };
}

/** The colour one preset contributes to one kind of theme, or `null` if unknown. */
export function presetColor(id: string, kind: ThemeKind): string | null {
  return ACCENT_PRESETS.find((preset) => preset.id === id)?.[kind] ?? null;
}

/** The amber preset, used when a stored accent names an id we do not know. */
const FALLBACK_ACCENT: Record<ThemeKind, string> = {
  dark: ACCENT_PRESETS[0].dark,
  light: ACCENT_PRESETS[0].light,
};

/**
 * The accent to apply, given the settings and the theme in force.
 *
 * A custom colour is one hex rather than a pair, so it is used as given in every
 * theme: the user picked it by looking at the screen in front of them, and
 * substituting a different value would be the app overruling that.
 */
export function accentColor(appearance: Appearance, theme: ThemeId): string {
  if (appearance.accent === "custom" && isHexColor(appearance.customAccent)) return appearance.customAccent;
  return presetColor(appearance.accent, themeKind(theme)) ?? FALLBACK_ACCENT[themeKind(theme)];
}

/** Relative luminance (WCAG 2.1) of a hex colour, 0..1. */
export function luminance(hex: string): number {
  const value = hex.replace("#", "");
  const channel = (offset: number) => {
    const srgb = parseInt(value.slice(offset, offset + 2), 16) / 255;
    return srgb <= 0.03928 ? srgb / 12.92 : ((srgb + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(0) + 0.7152 * channel(2) + 0.0722 * channel(4);
}

/**
 * Text that stays legible on top of `hex`.
 *
 * Computed rather than fixed, because a custom colour can be anything: a user
 * who picks a near-black accent would otherwise get near-black text on it and
 * lose the label on the send button entirely.
 */
export function inkFor(hex: string): string {
  return luminance(hex) > 0.45 ? "#1b1409" : "#ffffff";
}

/** Resolves the choice to the palette actually rendered. */
export function resolveTheme(choice: ThemeChoice): ThemeId {
  if (choice !== "system") return choice;
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}
