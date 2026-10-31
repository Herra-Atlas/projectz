/**
 * What the find bar searches, and how it reaches a match.
 *
 * **Scopes are declared, not guessed.** A scope is an element carrying
 * `data-find-scope`, so the app decides which surfaces text search may touch.
 * Searching the whole document would find the sidebar's session titles and the
 * composer's draft as readily as the reply the user is reading, and dimming
 * everything outside a match would take the navigation down with it. A new
 * surface becomes searchable by adding one attribute.
 *
 * `data-find-label` names the scope ("conversation", "file preview") and is what
 * the bar's placeholder reads, so a search says where it is looking.
 */
export type FindScope = { element: HTMLElement; label: string };

const SCOPE_SELECTOR = "[data-find-scope]";

/**
 * A box that draws one scope at a time.
 *
 * The right panel is one: it swaps the files tree for a file's preview, or for
 * the terminal, without being a searchable surface itself. A press is remembered
 * against its region as well as its scope, so opening a file from the tree -- or
 * switching the panel's view -- does not lose the fact that this is where the
 * user is working.
 */
const REGION_SELECTOR = "[data-find-region]";

/**
 * Subtrees whose text is never a match.
 *
 * A textarea's value is not a text node at all, an input's is an attribute, and
 * an SVG's text is a drawing. `aria-hidden` and `hidden` are text nobody is
 * reading. `[data-find-ui]` is the bar itself, in case a future scope is drawn
 * wide enough to contain the portal target; `[data-find-skip]` is the escape
 * hatch for anything else.
 */
const SKIP_SELECTOR =
  "script, style, textarea, input, select, option, svg, [hidden], [aria-hidden='true'], [data-find-skip], [data-find-ui]";

/**
 * Where the last press landed: the scope under it, and the region around it.
 *
 * Module state rather than a hook value because it describes the DOM, not one
 * component's render: `useFind` is a single instance for the whole app, and a
 * press anywhere -- including inside the right panel, which knows nothing about
 * find -- is worth remembering. A stale entry costs nothing: the scope is only
 * used while it is still visible, and the region while it still holds a visible
 * scope.
 */
let pressed: { scope: HTMLElement | null; region: Element | null } | null = null;

/**
 * Records where a press landed.
 *
 * The bar's own DOM is skipped, because it is portalled into wherever the top
 * layer is: without this, a press on the field could be read as a press on the
 * page it is drawn over.
 *
 * A press on chrome that belongs to a region -- the panel's tab strip, its header,
 * the back control above a file -- keeps that region and forgets the scope. That
 * is what makes "switch the panel to Files, then press Ctrl+F" search the tree
 * rather than asking the panel to be clicked a second time first.
 */
export function rememberPressedScope(target: EventTarget | null): void {
  if (!(target instanceof Element) || target.closest("[data-find-ui]")) return;
  pressed = {
    scope: target.closest<HTMLElement>(SCOPE_SELECTOR),
    region: target.closest(REGION_SELECTOR),
  };
}

/**
 * Scopes that are on screen right now.
 *
 * Visible means it has a box: the chat column is `hidden` while another view is
 * open, and the right panel is zero-width when closed, so `getClientRects` is
 * what keeps a search from counting text nobody can see.
 *
 * **A modal dialog wins.** While Settings is open the app behind it is still
 * painted, so both would be visible -- and a search in Settings that also dimmed
 * the conversation underneath would be highlighting a page the backdrop has
 * already hidden. When any scope lives inside an open dialog, only those are
 * searched.
 *
 * **Otherwise only the surface being worked in.** The panel is a dock, so the
 * chat stays visible beside it and two scopes are on screen most of the time;
 * searching both would count the file tree and the conversation as one list, and
 * Enter would walk out of the panel into a reply the user was not looking at.
 */
export function findScopes(): FindScope[] {
  const visible = Array.from(document.querySelectorAll<HTMLElement>(SCOPE_SELECTOR)).filter(isVisible);
  const dialog = document.querySelector("dialog[open]");
  const insideDialog = dialog ? visible.filter((element) => dialog.contains(element)) : [];
  const working = workingScope(visible);
  const chosen = insideDialog.length > 0 ? insideDialog : working ? [working] : visible;
  return chosen.map((element) => ({ element, label: element.dataset.findLabel ?? "page" }));
}

/**
 * The one scope the user is working in, or `null` when nothing says which.
 *
 * Focus is the first answer, because a control that holds the caret names its
 * surface exactly. It is not enough on its own: most of a panel is plain divs
 * and buttons, so pressing a tree's blank space leaves `document.activeElement`
 * on `body` and the press itself is the only evidence of where the user is.
 *
 * The press is then read in two steps. Its own scope if that element is still on
 * screen; otherwise the scope the pressed *region* draws now, because opening a
 * file from the tree or switching the panel's view replaces the element without
 * moving the user.
 *
 * `null` -- a fresh window, a press on the sidebar, or the caret in the composer,
 * which is not a scope -- falls back to every visible scope, which is what
 * someone who has touched nothing yet expects.
 */
function workingScope(visible: HTMLElement[]): HTMLElement | null {
  const active = document.activeElement;
  if (active instanceof Element) {
    const focused = visible.find((element) => element.contains(active));
    if (focused) return focused;
  }
  if (!pressed) return null;
  if (pressed.scope && visible.includes(pressed.scope)) return pressed.scope;
  const region = pressed.region;
  if (!region) return null;
  return visible.find((element) => region.contains(element)) ?? null;
}

function isVisible(element: HTMLElement): boolean {
  const rects = element.getClientRects();
  return rects.length > 0 && rects[0].width > 0 && rects[0].height > 0;
}

/** Text nodes under `root`, in document order, minus the subtrees above. */
export function textNodesIn(root: HTMLElement): Text[] {
  const nodes: Text[] = [];
  collect(root, nodes);
  return nodes;
}

function collect(node: Node, out: Text[]): void {
  for (let child = node.firstChild; child; child = child.nextSibling) {
    if (child.nodeType === Node.TEXT_NODE) {
      const text = child as Text;
      // Whitespace-only nodes are layout, not content, and would match a query of
      // a single space in every gap between elements.
      if (text.data.trim()) out.push(text);
      continue;
    }
    if (child.nodeType !== Node.ELEMENT_NODE) continue;
    // Pruning the subtree is the point: checking each text node's ancestors would
    // walk the same path once per node.
    if ((child as Element).matches(SKIP_SELECTOR)) continue;
    collect(child, out);
  }
}

/**
 * Scrolls a match into the middle of whatever is scrolling it.
 *
 * **Only when it is out of view.** Centring a match that is already comfortably
 * on screen would move the page under the reader's cursor on every Enter press.
 *
 * The scroller is found from the match upward rather than assumed to be the
 * scope: the chat thread scrolls itself, a Markdown preview scrolls its own
 * body, and both are marked as scopes. `behavior: "smooth"` is what makes the
 * jump a move; the global reduced-motion rule turns it into an instant scroll for
 * anyone who asked for that.
 */
export function revealMatch(range: Range, scope: HTMLElement): void {
  const rect = range.getBoundingClientRect();
  // A zero rect is text in a collapsed row or behind `display: none`; there is
  // nowhere to scroll to, and scrolling to the top-left would be a lie.
  if (rect.width === 0 && rect.height === 0) return;

  const scroller = scrollParentOf(range.startContainer, scope);
  const box = scroller.getBoundingClientRect();
  const margin = 48;
  if (rect.top >= box.top + margin && rect.bottom <= box.bottom - margin) return;

  const top = scroller.scrollTop + (rect.top - box.top) + rect.height / 2 - box.height / 2;
  const limit = Math.max(0, scroller.scrollHeight - scroller.clientHeight);
  scroller.scrollTo({ top: Math.min(Math.max(top, 0), limit), behavior: "smooth" });
}

/** The nearest ancestor that scrolls vertically, or the scope itself. */
function scrollParentOf(node: Node, scope: HTMLElement): HTMLElement {
  let element = node.parentElement;
  while (element) {
    const overflow = getComputedStyle(element).overflowY;
    if (overflow === "auto" || overflow === "scroll") return element;
    if (element === scope || element === document.body) break;
    element = element.parentElement;
  }
  return scope;
}
