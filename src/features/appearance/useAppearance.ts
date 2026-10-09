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
 * The mirror is written on every change, including the resolved colour for both
 * themes, so the pre-paint script can apply one without knowing the preset
 * table.
 */
export function useAppearance(enabled: boolean) {
  const [appearance, setAppearance] = useState<Appearance>(() => readMirror() ?? DEFAULT_APPEARANCE);

  // Adopt the stored record once the database is open. Until then the mirror --
  // or the default -- is what is on screen, which is the best answer available.
  useEffect(() => {
    if (!enabled) return;
    let mounted = true;
    invoke<Partial<Appearance> | null>("database_get_setting", { key: APPEARANCE_KEY })
      .then((saved) => {
        if (mounted && saved) setAppearance(normalizeAppearance(saved));
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

      // The accent resolved for every palette, so the pre-paint script can apply
      // the right one for whichever theme it lands on without knowing the preset
      // table or which themes are light. Built from the theme list rather than
      // written out, so a theme added to `THEMES` is covered without a second
      // edit here -- the failure that would cause is a launch in the wrong
      // accent, which is exactly the kind of thing nobody would notice.
      const themes = Object.fromEntries(
        THEMES.map(({ id }) => {
          const color = accentColor(appearance, id);
          return [id, { accent: color, ink: inkFor(color) }];
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
    void invoke("database_set_setting", { key: APPEARANCE_KEY, value: next })
      .catch((reason: unknown) => {
        // The value is already on screen and will be again next launch if the
        // write succeeds then, so this is logged rather than raised.
        console.error("Appearance could not be saved:", reason);
      });
  }, []);

  return { appearance, saveAppearance };
}
