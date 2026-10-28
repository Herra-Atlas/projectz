/**
 * Painting matches with the CSS Custom Highlight API.
 *
 * **Ranges, not `<mark>` elements.** Wrapping matches in nodes would mean
 * mutating the DOM React owns: a re-render would drop the wrappers mid-search,
 * and `Markdown` is memoised on its text, so it would not put them back. The
 * Highlight API paints a `Range` without touching the tree, which is also why a
 * keystroke costs one registry write rather than a re-render of the transcript.
 *
 * Two highlights rather than one: every match gets a soft accent wash, and the
 * current match is registered separately at a higher priority so it paints over
 * the top of its own softer copy. Moving to the next match then rewrites one
 * range instead of the whole set.
 *
 * The styles live in `App.css` (`::highlight(pz-find-match)`), because a
 * highlight pseudo-element cannot be written as a utility class.
 */

const MATCH = "pz-find-match";
const CURRENT = "pz-find-current";

/** Where the dim is written. An attribute rather than a class, because React
    owns `className` on the elements scopes are drawn from. */
const DIM_ATTRIBUTE = "data-find-dim";

/**
 * Whether this webview can paint highlights at all.
 *
 * WebView2 is evergreen Chromium and WKWebView 17.2+ has the API, so this is
 * true everywhere the app ships. It is still checked: without it the search must
 * fall back to counting and scrolling, not to dimming a page whose matches it
 * cannot light up -- that would make the text unreadable for no gain.
 */
export const HIGHLIGHTS_SUPPORTED =
  typeof CSS !== "undefined" && "highlights" in CSS && typeof Highlight !== "undefined";

/** The scopes currently dimmed, so the next call can undo exactly its own work. */
let dimmed: HTMLElement[] = [];

/** Paints every match, with `active` painted on top. */
export function paintMatches(ranges: Range[], active: number): void {
  if (!HIGHLIGHTS_SUPPORTED) return;
  CSS.highlights.set(MATCH, new Highlight(...ranges));
  setCurrentHighlight(ranges[active]);
}

/**
 * Paints only the current match.
 *
 * Used when stepping: the set of matches has not changed, so rebuilding it would
 * allocate a `Highlight` over thousands of ranges to move one of them.
 */
export function setCurrentHighlight(range: Range | undefined): void {
  if (!HIGHLIGHTS_SUPPORTED) return;
  if (!range) {
    CSS.highlights.delete(CURRENT);
    return;
  }
  const highlight = new Highlight(range);
  highlight.priority = 1;
  CSS.highlights.set(CURRENT, highlight);
}

/**
 * Removes the paint and the dim.
 *
 * Called on every close and on unmount, so a conversation is never left faded
 * because a find bar was dismissed -- the dim is a mode of looking, not a state
 * of the page.
 */
export function clearHighlights(): void {
  if (HIGHLIGHTS_SUPPORTED) {
    CSS.highlights.delete(MATCH);
    CSS.highlights.delete(CURRENT);
  }
  setDimmedScopes([], false);
}

/** Fades every scope but keeps the matches legible, or restores them. */
export function setDimmedScopes(scopes: HTMLElement[], dim: boolean): void {
  for (const element of dimmed) element.removeAttribute(DIM_ATTRIBUTE);
  dimmed = dim ? scopes : [];
  for (const element of dimmed) element.setAttribute(DIM_ATTRIBUTE, "");
}
