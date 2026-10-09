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
 * A palette the app can render in. Each one has a matching `[data-theme=...]`
 * rule in `App.css`, and adding one means adding it in both places.
 */
export type ThemeId = "dark" | "midnight" | "slate" | "forest" | "plum" | "light" | "paper" | "mist";

/** What the user chose. `system` follows the operating system. */
export type ThemeChoice = ThemeId | "system";

/**
 * Which half of the palette scale a theme belongs to.
 *
 * Only used to pick an accent: a colour legible as text on near-black is not
 * legible as text on white, so each accent preset carries a value for each half
 * and a theme says which half it is. A new theme therefore costs no new accent
 * values -- it declares which side it is on.
 */
export type ThemeKind = "dark" | "light";

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

/** The themes offered, in the order the picker shows them. */
export const THEMES: { id: ThemeId; label: string }[] = [
  { id: "dark", label: "Dark" },
  { id: "midnight", label: "Midnight" },
  { id: "slate", label: "Slate" },
  { id: "forest", label: "Forest" },
  { id: "plum", label: "Plum" },
  { id: "light", label: "Light" },
  { id: "paper", label: "Paper" },
  { id: "mist", label: "Mist" },
];

/** Every value `theme` may hold, for validating what was stored. */
const THEME_CHOICES = new Set<string>([...THEMES.map((theme) => theme.id), "system"]);

/** The themes whose ground is light. Named once, so `themeKind` cannot come to
    a different conclusion about a theme than the list above it. */
const LIGHT_THEMES: ThemeId[] = ["light", "paper", "mist"];

/** Which half of the accent scale a theme draws from. */
export function themeKind(theme: ThemeId): ThemeKind {
  return LIGHT_THEMES.includes(theme) ? "light" : "dark";
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
