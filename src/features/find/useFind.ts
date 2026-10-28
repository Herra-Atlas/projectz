import { useCallback, useEffect, useRef, useState } from "react";
import { loadFindOptions, saveFindOptions, type FindOptions } from "./options";
import { matchText } from "./matcher";
import { findScopes, revealMatch, textNodesIn } from "./scopes";
import { HIGHLIGHTS_SUPPORTED, clearHighlights, paintMatches, setCurrentHighlight, setDimmedScopes } from "./highlights";

/** One match and the scope it was found in, which is where it scrolls. */
type FindMatch = { range: Range; scope: HTMLElement };

export type FindController = {
  open: boolean;
  query: string;
  options: FindOptions;
  /** Zero-based index of the current match, into `total`. */
  active: number;
  total: number;
  /** True when `total` is the cap rather than the whole count. */
  truncated: boolean;
  /** What the bar is searching ("conversation", "file preview"). */
  label: string;
  /** Increments on every Ctrl+F, so pressing it again re-selects the field. */
  focusTick: number;
  show: () => void;
  close: () => void;
  setQuery: (next: string) => void;
  setOptions: (next: FindOptions) => void;
  step: (direction: 1 | -1) => void;
};

/** Typing pause before the page is scanned again. Long enough to skip the
 *  intermediate strings of a typed word, short enough to feel immediate. */
const SCAN_DEBOUNCE_MS = 80;

/** How long the page must sit still before a live reply is re-scanned. A
 *  streaming answer mutates its text many times a second, and re-walking a long
 *  transcript on each of those would be work nobody sees. */
const RESCAN_DEBOUNCE_MS = 250;

/**
 * The most matches the bar will hold.
 *
 * Each match is a live `Range`, and a one-letter query in a long conversation can
 * match tens of thousands of times. Past this the count says `2000+`: the reader
 * wants to know the page is full of hits and to keep stepping through them, not
 * an exact figure that costs a second to compute.
 *
 * Exported because the bar names the same number in its tooltip; two copies of
 * "what the cap is" would drift.
 */
export const FIND_MATCH_CAP = 2000;

/**
 * The app's find.
 *
 * Owns the query, the switches, the match list and the painting. The bar is only
 * a view over this: one instance lives in `App`, so a search survives the bar
 * being closed and reopened, and the keyboard shortcut works on every surface
 * that declared a scope.
 *
 * **Ranges are held in a ref, not in state.** They are not values React can
 * compare or should render, and a state update per match would re-render the
 * whole bar on every keystroke. Only what the bar draws -- the count, the index,
 * the label -- is state.
 */
export function useFind(): FindController {
  const [open, setOpen] = useState(false);
  const [query, setQueryState] = useState("");
  const [options, setOptionsState] = useState<FindOptions>(loadFindOptions);
  const [active, setActive] = useState(0);
  const [total, setTotal] = useState(0);
  const [truncated, setTruncated] = useState(false);
  const [label, setLabel] = useState("page");
  const [focusTick, setFocusTick] = useState(0);
  /** Bumped when the set of scopes changes, so the observer attaches to the
      surface that is actually on screen. */
  const [scopeTick, setScopeTick] = useState(0);

  const matchesRef = useRef<FindMatch[]>([]);
  const activeRef = useRef(0);
  const openRef = useRef(false);
  const queryRef = useRef("");
  const optionsRef = useRef(options);
  const scopesRef = useRef<HTMLElement[]>([]);
  const previousFocusRef = useRef<HTMLElement | null>(null);

  // Mirrored during render, so the timers and listeners below read the newest
  // values without being re-created on every keystroke.
  openRef.current = open;
  queryRef.current = query;
  optionsRef.current = options;

  /** Puts a match list on screen, clamped to the list that actually exists. */
  const apply = useCallback((matches: FindMatch[], index: number, reveal: boolean) => {
    matchesRef.current = matches;
    const next = matches.length === 0 ? 0 : Math.min(Math.max(index, 0), matches.length - 1);
    activeRef.current = next;
    setActive(next);
    setTotal(matches.length);
    paintMatches(matches.map((match) => match.range), next);
    // Dim only when something can be lit up. Without highlights the page would
    // fade with nothing standing out of it.
    setDimmedScopes(scopesRef.current, HIGHLIGHTS_SUPPORTED && matches.length > 0);
    if (!reveal) return;
    const match = matches[next];
    if (match) revealMatch(match.range, match.scope);
  }, []);

  /** Walks the visible scopes and rebuilds the match list. */
  const scan = useCallback((reveal: boolean) => {
    const scopes = findScopes();
    const elements = scopes.map((scope) => scope.element);
    const moved =
      elements.length !== scopesRef.current.length ||
      elements.some((element, index) => element !== scopesRef.current[index]);
    scopesRef.current = elements;
    if (moved) setScopeTick((tick) => tick + 1);
    setLabel(scopes[0]?.label ?? "page");

    const text = queryRef.current;
    if (!text || scopes.length === 0) {
      setTruncated(false);
      apply([], 0, false);
      return;
    }

    const matches: FindMatch[] = [];
    let cut = false;
    for (const scope of scopes) {
      for (const node of textNodesIn(scope.element)) {
        for (const found of matchText(node.data, text, optionsRef.current)) {
          if (matches.length >= FIND_MATCH_CAP) {
            cut = true;
            break;
          }
          const range = document.createRange();
          range.setStart(node, found.start);
          range.setEnd(node, found.end);
          matches.push({ range, scope: scope.element });
        }
        if (cut) break;
      }
      if (cut) break;
    }

    setTruncated(cut);
    apply(matches, activeRef.current, reveal);
  }, [apply]);

  /**
   * Re-scan on a new query, a changed switch, or the bar opening.
   *
   * The index goes back to the first match: a new search is a new question, and
   * keeping a position from the old one would put the reader in the middle of an
   * answer they had not seen yet.
   */
  useEffect(() => {
    if (!open) return;
    activeRef.current = 0;
    const timer = window.setTimeout(() => scan(true), SCAN_DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [open, query, options, scan]);

  /**
   * Follows the page while the bar is open.
   *
   * A reply that is still streaming, or a conversation that finishes loading,
   * changes the text under a search that is already showing a count. Re-walking
   * on a short debounce keeps the figure honest; `reveal` is false because the
   * reader is watching the answer arrive and must not be dragged back to an old
   * match by every chunk.
   */
  useEffect(() => {
    if (!open || !query) return;
    const scopes = scopesRef.current;
    if (scopes.length === 0) return;
    let timer: number | undefined;
    const observer = new MutationObserver(() => {
      window.clearTimeout(timer);
      timer = window.setTimeout(() => scan(false), RESCAN_DEBOUNCE_MS);
    });
    for (const scope of scopes) {
      observer.observe(scope, { childList: true, characterData: true, subtree: true });
    }
    return () => {
      observer.disconnect();
      window.clearTimeout(timer);
    };
  }, [open, query, scopeTick, scan]);

  const show = useCallback(() => {
    if (!openRef.current) {
      previousFocusRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    }
    // Written now as well as on the next render, so a second key event in the
    // same tick already sees the bar as open.
    openRef.current = true;
    setFocusTick((tick) => tick + 1);
    setOpen(true);
  }, []);

  const close = useCallback(() => {
    if (!openRef.current) return;
    openRef.current = false;
    setOpen(false);
    // The query is kept, as every find bar does: reopening continues the search
    // the user was already in rather than starting from an empty field.
    matchesRef.current = [];
    activeRef.current = 0;
    setActive(0);
    setTotal(0);
    setTruncated(false);
    clearHighlights();
    // Back to whatever had the caret before the search. Focus left on a hidden
    // field is focus the keyboard cannot get out of.
    const previous = previousFocusRef.current;
    previousFocusRef.current = null;
    if (previous?.isConnected) previous.focus();
  }, []);

  const step = useCallback((direction: 1 | -1) => {
    const matches = matchesRef.current;
    if (matches.length === 0) return;
    const next = (activeRef.current + direction + matches.length) % matches.length;
    activeRef.current = next;
    setActive(next);
    const match = matches[next];
    setCurrentHighlight(match?.range);
    if (match) revealMatch(match.range, match.scope);
  }, []);

  const setQuery = useCallback((next: string) => setQueryState(next), []);

  const setOptions = useCallback((next: FindOptions) => {
    setOptionsState(next);
    saveFindOptions(next);
  }, []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      // Ctrl+F belongs to the app now: the webview's browser accelerators are
      // turned off at startup (`src-tauri/src/webview_settings.rs`), so nothing
      // else would answer it. The physical `code` is checked beside `key` because
      // a Cyrillic or Greek layout reports `key: "а"`/`"φ"` for the same press.
      if ((event.ctrlKey || event.metaKey) && !event.altKey && !event.shiftKey && (event.key.toLowerCase() === "f" || event.code === "KeyF")) {
        event.preventDefault();
        event.stopPropagation();
        show();
        return;
      }
      // F3 is the other one the browser would have answered. Only while the bar
      // is open, so it stays a shortcut of this feature rather than of the app.
      if (openRef.current && event.key === "F3") {
        event.preventDefault();
        step(event.shiftKey ? -1 : 1);
      }
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [show, step]);

  // A search never outlives the bar: the dim and the paint are removed with it.
  useEffect(() => clearHighlights, []);

  return {
    open,
    query,
    options,
    active,
    total,
    truncated,
    label,
    focusTick,
    show,
    close,
    setQuery,
    setOptions,
    step,
  };
}
