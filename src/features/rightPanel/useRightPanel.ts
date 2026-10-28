import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

/**
 * Settings key holding the right panel's own state.
 *
 * `app.right_panel` rather than localStorage: the app keeps every durable
 * preference in SQLite under an `app.*` key, written through
 * `database_set_setting`, so a value here is readable from the Database page
 * like everything else. One store means one answer to "where does this live".
 */
export const RIGHT_PANEL_KEY = "app.right_panel";

/**
 * Width bounds, in logical pixels.
 *
 * The floor is roughly the narrowest a file tree stays readable. There is no
 * fixed ceiling: the panel may take at most **half the window it is docked in**,
 * which is the one rule that holds on a laptop and on a wide monitor alike. A
 * fixed number was the complaint -- a large window still could not give the panel
 * more than 720, and a smaller one could still be half-swallowed.
 */
export const MIN_PANEL_WIDTH = 260;
export const DEFAULT_PANEL_WIDTH = 380;

/** The widest the panel may be in a window `windowWidth` logical pixels wide. */
export function maxPanelWidth(windowWidth = window.innerWidth): number {
  return Math.max(MIN_PANEL_WIDTH, Math.round(windowWidth / 2));
}

/**
 * Clamps a width into range.
 *
 * The window is read when this is called, not captured with the caller: a drag,
 * a load and a resize all need the *current* window, and a ceiling remembered
 * from an earlier render is the ceiling of a window that may since have changed
 * size.
 */
export function clampPanelWidth(width: number, windowWidth = window.innerWidth): number {
  return Math.min(maxPanelWidth(windowWidth), Math.max(MIN_PANEL_WIDTH, Math.round(width)));
}

/**
 * Coerces a stored value into range, or to the default when it is nonsense.
 *
 * Sanitising on *load* rather than only on edit, because a width stored against
 * an older build -- or against a larger window -- can sit outside the current
 * bounds, and a panel that opens off-screen is worse than one that ignores the
 * number.
 */
function sanitizeWidth(value: unknown): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return DEFAULT_PANEL_WIDTH;
  return clampPanelWidth(value);
}

export type RightPanelState = {
  /** Whether the panel holds any width. False renders it at zero. */
  open: boolean;
  /** Current width, retained even while closed so reopening restores it. */
  width: number;
};

const DEFAULT_STATE: RightPanelState = { open: false, width: DEFAULT_PANEL_WIDTH };

export function useRightPanel() {
  const [state, setState] = useState<RightPanelState>(DEFAULT_STATE);

  useEffect(() => {
    let mounted = true;
    invoke<Partial<RightPanelState> | null>("database_get_setting", { key: RIGHT_PANEL_KEY })
      .then((saved) => {
        if (!mounted || !saved) return;
        setState({ open: saved.open === true, width: sanitizeWidth(saved.width) });
      })
      // No stored value yet, or a database that predates the panel. The defaults
      // are the honest starting point, so there is nothing to report.
      .catch(() => undefined);
    return () => { mounted = false; };
  }, []);

  /**
   * Keeps the panel inside a window that has just shrunk.
   *
   * Half the window is a moving ceiling, so a panel that was legitimately wide on
   * a maximised window must give width back when the window narrows -- otherwise
   * the rule only holds until the first resize. It only ever narrows the panel:
   * a window that grows leaves the width the user chose alone, because taking
   * half of a newly huge screen is not what they asked for when they dragged.
   */
  useEffect(() => {
    const fit = () =>
      setState((current) => {
        const width = clampPanelWidth(current.width);
        return width === current.width ? current : { ...current, width };
      });
    fit();
    window.addEventListener("resize", fit);
    return () => window.removeEventListener("resize", fit);
  }, []);

  /**
   * Writes are debounced, because a drag raises a pointer event every frame and
   * a write per frame is hundreds of SQLite round trips for one gesture. The
   * first render is skipped: it is the *load*, not a change, and writing there
   * would immediately overwrite whatever was just read.
   *
   * The timer is cleared rather than flushed on unmount, because the only way
   * to unmount is to close the app, and a write that does not land loses a
   * width the user set seconds ago. Everything else is a drag still in progress
   * whose next pointer event re-arms the timer anyway.
   */
  const loadedRef = useRef(false);
  // Mirrored so the timer always reads the newest value without the effect
  // having to depend on `state` and re-arm on every frame of a drag.
  const latestRef = useRef(state);
  latestRef.current = state;
  useEffect(() => {
    if (!loadedRef.current) {
      loadedRef.current = true;
      return;
    }
    const timer = window.setTimeout(() => {
      void invoke("database_set_setting", { key: RIGHT_PANEL_KEY, value: latestRef.current })
        .catch((reason: unknown) => console.error("Right panel state could not be saved:", reason));
    }, 400);
    return () => window.clearTimeout(timer);
  }, [state]);

  const setOpen = useCallback((open: boolean) => {
    setState((current) => (current.open === open ? current : { ...current, open }));
  }, []);

  const toggle = useCallback(() => {
    setState((current) => ({ ...current, open: !current.open }));
  }, []);

  const setWidth = useCallback((width: number) => {
    setState((current) => ({ ...current, width: sanitizeWidth(width) }));
  }, []);

  return { ...state, setOpen, toggle, setWidth };
}