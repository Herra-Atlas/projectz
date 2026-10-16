import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Clipboard, Globe, ListTree, Sparkles } from "lucide-react";
import { urlFromSelection } from "./urls";

export type SelectionAction = "explain" | "summarize";

type SelectionActionsProps = {
  /** The text the user highlighted, captured before the toolbar click. */
  text: string;
  /** Viewport coordinates of the selection, used to anchor the bar. */
  top: number;
  left: number;
  onCopy: () => void;
  onAction: (action: SelectionAction) => void;
  /**
   * Opens the selection's own address in the panel's browser.
   *
   * Only ever called when the highlight *is* a URL -- the bar hides the button
   * otherwise -- so the caller needs no check of its own.
   */
  onOpenUrl: (url: string) => void;
  onDismiss: () => void;
};

const TOOLBAR_HEIGHT = 36;
const GAP = 8;

/**
 * Floating bar shown when text inside an assistant reply is selected.
 *
 * Copy acts immediately and the two drafting actions hand the selection back to
 * the caller so the composer can be pre-filled for review before anything is
 * sent. Open appears only when the highlight is a URL, and hands it to the panel
 * browser. Anchored above the selection and flipped below when there is no room.
 */
export default function SelectionActions({ text, top, left, onCopy, onAction, onOpenUrl, onDismiss }: SelectionActionsProps) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => { if (event.key === "Escape") onDismiss(); };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [onDismiss]);

  // The selection's own address, when it is one. Its presence is the only thing
  // that decides whether the bar offers to open it.
  const url = urlFromSelection(text);

  // Width is measured after mount so the bar can be centred on the selection.
  // Re-measured if the Open button appears or disappears, since a bar that keeps
  // a width from the previous selection would be centred slightly wrong.
  const [width, setWidth] = useState(0);
  useEffect(() => { setWidth(ref.current?.offsetWidth ?? 0); }, [url !== null]);
  const above = top - TOOLBAR_HEIGHT - GAP;
  const offset = width > 0 ? Math.max(8, left - width / 2) : left;

  return createPortal(
    <div
      ref={ref}
      role="toolbar"
      aria-label="Selection actions"
      // Named so the page's outside-click dismissal can tell this bar from the
      // rest of the page. Without it, pressing a button here would dismiss the
      // bar before the click landed on it.
      data-selection-bar=""
      style={{ top: above >= 8 ? above : top + GAP, left: offset }}
      // Keep the browser from clearing the selection when the bar is pressed.
      onMouseDown={(event) => event.preventDefault()}
      className="fixed z-[130] flex h-9 items-center gap-0.5 rounded-lg border border-[var(--line)] bg-[var(--rail)] p-1 shadow-2xl"
    >
      <button type="button" onClick={onCopy} title="Copy selection" aria-label="Copy selection" className="grid size-7 place-items-center rounded-md text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)]"><Clipboard size={14} /></button>
      <span aria-hidden="true" className="mx-0.5 h-4 w-px bg-[var(--line)]" />
      <button type="button" onClick={() => onAction("explain")} title="Draft an explanation" aria-label="Explain selection" className="inline-flex h-7 items-center gap-1.5 rounded-md px-2 text-xs text-[var(--text)] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)]"><Sparkles size={13} className="text-[var(--muted)]" />Explain</button>
      <button type="button" onClick={() => onAction("summarize")} title="Draft a summary" aria-label="Summarize selection" className="inline-flex h-7 items-center gap-1.5 rounded-md px-2 text-xs text-[var(--text)] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)]"><ListTree size={13} className="text-[var(--muted)]" />Summarize</button>
      {/* Offered only for a highlight that is a URL of its own, so the bar is
          unchanged for ordinary prose. */}
      {url && <button type="button" onClick={() => onOpenUrl(url)} title="Open in the browser panel" aria-label="Open link in the browser panel" className="inline-flex h-7 items-center gap-1.5 rounded-md px-2 text-xs text-[var(--text)] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)]"><Globe size={13} className="text-[var(--muted)]" />Open</button>}
      <span className="sr-only">{text.length} characters selected</span>
    </div>,
    document.body,
  );
}
