import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  APPEARANCE_KEY,
  APPEARANCE_STORAGE_KEY,
  DEFAULT_APPEARANCE,
  THEMES,
  accentColor,
  inkFor,
  normalizeAppearance,
  resolveTheme,
  themeColor,
  windowTheme,
  type Appearance,
} from "./appearance";

/** What the pre-paint script left behind, so the first render agrees with it. */
function readMirror(): Appearance | null {
  try {
    const saved = JSON.parse(localStorage.getItem(APPEARANCE_STORAGE_KEY) ?? "null");
    if (!saved || typeof saved !== "object") return null;
    return normalizeAppearance(saved as Partial<Appearance>);
  } catch {
    return null;
  }
}

/**
 * A stored record, which carries one field the record in memory does not: the
 * frame theme the native window is painted from before the frontend runs.
 */
type StoredAppearance = Partial<Appearance> & { windowTheme?: string };

/**
 * Writes the record, with the frame theme folded in.
 *
 * The frame value is derived from the choice, but stored *beside* the record
 * rather than computed in Rust: the native title bar is drawn the moment the
 * window appears, before any frontend code has run, and `window.rs` reads this
 * to colour it. Resolving it here is what keeps Rust from needing a second list
 * of which themes are light -- the same duplication, across a language boundary,
 * that the theme list on this side was just collapsed to avoid.
 */
function persistAppearance(next: Appearance) {
  void invoke("database_set_setting", {
    key: APPEARANCE_KEY,
    value: { ...next, windowTheme: windowTheme(next.theme) },
  }).catch((reason: unknown) => {
    // The value is already on screen and will be again next launch if the write
    // succeeds then, so this is logged rather than raised.
    console.error("Appearance could not be saved:", reason);
  });
}

/**
 * Applies the theme and accent, and persists them.
 *
 * **Applied to the document element, not passed down.** The palette is a set of
 * custom properties that every screen already reads, so switching themes is one
 * write of `data-theme` on `<html>` rather than a prop threaded through the
 * tree. That is also what makes the change cover surfaces this component has
 * never heard of -- a chart, a diff, the scrollbar.
 *
 * **The database is the record; `localStorage` is the mirror.** The stored value
 * is read once the database is up, but the initial state comes from the mirror
 * so the app paints in the right theme immediately. Without it a light theme
 * would open dark and correct itself a moment later, on every launch.
 *
 * The mirror is written on every change, including the resolved colour for every
 * theme, so the pre-paint script can apply one without knowing the preset table.
 *
 * **The native frame is told separately.** The title bar and its buttons are the
 * OS's, not the webview's, so no token reaches them; the resolved light/dark half
 * is sent through `set_window_theme`, and stored with the record so `window.rs`
 * can paint it on the next launch before this hook has run.
 */
export function useAppearance(enabled: boolean) {
  const [appearance, setAppearance] = useState<Appearance>(() => readMirror() ?? DEFAULT_APPEARANCE);

  // Adopt the stored record once the database is open. Until then the mirror --
  // or the default -- is what is on screen, which is the best answer available.
  useEffect(() => {
    if (!enabled) return;
    let mounted = true;
    invoke<StoredAppearance | null>("database_get_setting", { key: APPEARANCE_KEY })
      .then((saved) => {
        if (!mounted || !saved) return;
        const next = normalizeAppearance(saved);
        setAppearance(next);
        // A record saved before the frame theme was stored has no value for it,
        // so every launch would open under the system frame until the user next
        // changed something. Write it back once here so the next launch is right.
        if (saved.windowTheme !== windowTheme(next.theme)) persistAppearance(next);
      })
      .catch(() => undefined);
    return () => { mounted = false; };
  }, [enabled]);

  useEffect(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");

    const apply = () => {
      const theme = resolveTheme(appearance.theme);
      const accent = accentColor(appearance, theme);
      const root = document.documentElement;
      root.dataset.theme = theme;
      root.style.setProperty("--accent", accent);
      root.style.setProperty("--accent-ink", inkFor(accent));
      // Kept in step with the theme after the first paint too. The pre-paint
      // script can only set it once, so without this a switch from a dark to a
      // light palette would leave the window frame tinted for the old one.
      const meta = document.querySelector('meta[name="theme-color"]');
      if (meta) meta.setAttribute("content", themeColor(theme));

      // The frame -- the title bar and its buttons -- is drawn by the OS, so it
      // is the one surface that cannot be a CSS token and is set through a
      // command instead. Light or dark, never a palette: the frame has no
      // opinion about which of the themes is in force.
      void invoke("set_window_theme", { theme: windowTheme(appearance.theme) }).catch(() => undefined);

      // The accent and the window tint resolved for every palette, so the
      // pre-paint script can apply the right ones for whichever theme it lands on
      // without knowing the preset table or which themes are light. Built from
      // the theme list rather than written out, so a theme added to `THEMES` is
      // covered without a second edit here -- the failure that would cause is a
      // launch in the wrong accent, which is exactly the kind of thing nobody
      // would notice.
      const themes = Object.fromEntries(
        THEMES.map(({ id }) => {
          const color = accentColor(appearance, id);
          return [id, { accent: color, ink: inkFor(color), themeColor: themeColor(id) }];
        }),
      );
      try {
        localStorage.setItem(APPEARANCE_STORAGE_KEY, JSON.stringify({ ...appearance, themes }));
      } catch {
        // A full quota costs the next launch its flash-free start and nothing
        // else. Not worth surfacing, and definitely not worth failing to paint.
      }
    };

    apply();
    // Only a `system` choice has anything to react to; a pinned theme does not
    // care what the operating system does.
    if (appearance.theme !== "system") return;
    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, [appearance]);

  const saveAppearance = useCallback((next: Appearance) => {
    setAppearance(next);
    persistAppearance(next);
  }, []);

  return { appearance, saveAppearance };
}
