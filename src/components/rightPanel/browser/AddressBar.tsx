import { useEffect, useRef, useState } from "react";
import { ArrowLeft, ArrowRight, Ellipsis, LoaderCircle, RefreshCw, Search } from "lucide-react";

/**
 * The browser's toolbar: back, forward, reload, the address field, and the two
 * controls that act on the page rather than the panel.
 *
 * Its own file rather than part of `BrowserTab` because it holds real state --
 * what is typed, whether the field is focused, whether a menu is open -- and none
 * of that belongs in the view that positions a webview. The two are separate
 * because the view is about *where the page is* and this is about *what the user
 * does to it*.
 */

type AddressBarProps = {
  /** The address shown, or empty before the first navigation. */
  url: string;
  canGoBack: boolean;
  canGoForward: boolean;
  loading: boolean;
  onNavigate: (input: string) => Promise<void>;
  onReload: () => void;
  onBack: () => void;
  onForward: () => void;
  onOpenExternally: () => Promise<void>;
  /** Reported when an address cannot be opened, so the bar can show why. */
  onError?: (message: string) => void;
};

export default function AddressBar({
  url,
  canGoBack,
  canGoForward,
  loading,
  onNavigate,
  onReload,
  onBack,
  onForward,
  onOpenExternally,
  onError,
}: AddressBarProps) {
  /**
   * What is typed, held separately from `url`.
   *
   * Editing an address must not navigate the browser on every keystroke, so the
   * field needs its own value. It is seeded from the loaded page and only
   * re-seeded when the field is *not* being edited -- otherwise a page that
   * finishes loading would overwrite the half-typed address the user is looking
   * at, which is the failure that makes a browser bar unusable.
   */
  const [draft, setDraft] = useState(url);
  const [editing, setEditing] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);
  const fieldRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!editing) setDraft(url);
  }, [url, editing]);

  const submit = async () => {
    const input = draft.trim();
    if (!input) return;
    try {
      await onNavigate(input);
      // Only released once the navigation was accepted. A refused address leaves
      // the field focused and the text in place, so the fix is one edit away
      // rather than retyped.
      setEditing(false);
      fieldRef.current?.blur();
    } catch (reason) {
      onError?.(String(reason));
    }
  };

  // `n` focuses the bar the way a browser's does, but only when nothing else has
  // focus and there is no modifier held -- a bare `n` typed into a text field is
  // an `n`, and this bar is not the only field in the app.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "n" || event.ctrlKey || event.metaKey || event.altKey) return;
      const active = document.activeElement;
      if (active instanceof HTMLElement && (active.tagName === "INPUT" || active.tagName === "TEXTAREA" || active.isContentEditable)) {
        return;
      }
      event.preventDefault();
      fieldRef.current?.focus();
      fieldRef.current?.select();
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, []);

  // The menu closes on an outside click or Escape, and the listeners exist only
  // while it is open, so a closed menu costs nothing.
  useEffect(() => {
    if (!menuOpen) return;
    const dismiss = (event: MouseEvent) => {
      const target = event.target as Node;
      if (target instanceof Element && target.closest("[data-browser-menu]")) return;
      setMenuOpen(false);
    };
    const dismissEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setMenuOpen(false);
    };
    document.addEventListener("pointerdown", dismiss);
    document.addEventListener("keydown", dismissEscape);
    return () => {
      document.removeEventListener("pointerdown", dismiss);
      document.removeEventListener("keydown", dismissEscape);
    };
  }, [menuOpen]);

  // The bar's own button, so the three navigation controls read as one set rather
  // than three unrelated marks.
  const navButton =
    "grid size-7 shrink-0 place-items-center rounded-md text-[var(--muted)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)] disabled:pointer-events-none disabled:opacity-35";

  return (
    <div className="flex shrink-0 items-center gap-1 border-b border-[var(--line)] px-2 py-1.5">
      <button
        type="button"
        onClick={onBack}
        disabled={!canGoBack}
        // `aria-disabled` rather than `disabled` for the two that are simply
        // inactive: a greyed-out control reads as broken, and this one is not --
        // there is simply nothing behind it to go to.
        aria-label="Back"
        title="Back"
        className={navButton}
      >
        <ArrowLeft size={15} />
      </button>
      <button type="button" onClick={onForward} disabled={!canGoForward} aria-label="Forward" title="Forward" className={navButton}>
        <ArrowRight size={15} />
      </button>
      <button
        type="button"
        onClick={onReload}
        disabled={!url}
        aria-label="Reload"
        title="Reload"
        className={navButton}
      >
        {/* The spinner replaces the icon while loading rather than sitting beside
            it, so the control does not change width mid-navigation and the row
            does not shuffle. */}
        {loading ? <LoaderCircle size={15} className="animate-spin text-[var(--accent)]" /> : <RefreshCw size={15} />}
      </button>

      {/* One field for both an address and a search, because that is what a
          browser's address bar is. The backend decides which one it is: a phrase
          is searched, anything URL-shaped is opened. */}
      <form
        className="min-w-0 flex-1"
        onSubmit={(event) => {
          event.preventDefault();
          void submit();
        }}
      >
        <div className="flex items-center gap-1.5 rounded-md bg-[var(--raised)] px-2.5 py-1.5 transition-colors focus-within:ring-1 focus-within:ring-[var(--accent)]">
          <Search size={13} className="shrink-0 text-[var(--quiet)]" />
          <input
            ref={fieldRef}
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            onFocus={(event) => {
              setEditing(true);
              // Select-all on focus, as a browser does: the commonest thing to
              // do with an address bar is replace it, and a caret in the middle
              // of a long URL makes that an editing task.
              event.target.select();
            }}
            onBlur={() => setEditing(false)}
            placeholder="Search or enter address"
            aria-label="Search or enter address"
            spellCheck={false}
            autoComplete="off"
            className="min-w-0 flex-1 bg-transparent text-xs text-[var(--text)] outline-none placeholder:text-[var(--quiet)]"
          />
        </div>
      </form>

      <button
        type="button"
        onClick={() => void onOpenExternally()}
        disabled={!url}
        aria-label="Open in your browser"
        title="Open in your browser"
        className={navButton}
      >
        {/* The external-link mark, which already means "leaves this app" wherever
            the app has used it. A label would not fit at this width. */}
        <ExternalMark />
      </button>

      <div className="relative shrink-0" data-browser-menu>
        <button
          type="button"
          onClick={() => setMenuOpen((open) => !open)}
          aria-label="Browser options"
          title="Browser options"
          aria-haspopup="menu"
          aria-expanded={menuOpen}
          className={navButton}
        >
          <Ellipsis size={15} />
        </button>
        {menuOpen && (
          <div role="menu" aria-label="Browser options" className="absolute right-0 top-full z-50 mt-1 w-52 rounded-xl border border-[var(--line)] bg-[var(--rail)] p-1.5 shadow-2xl">
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                setMenuOpen(false);
                void onOpenExternally();
              }}
              disabled={!url}
              className="flex min-h-9 w-full items-center rounded-lg px-2.5 text-left text-[13px] text-[var(--text)] transition-colors hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)] disabled:pointer-events-none disabled:opacity-40"
            >
              Open in your browser
            </button>
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                setMenuOpen(false);
                fieldRef.current?.focus();
                fieldRef.current?.select();
              }}
              className="flex min-h-9 w-full items-center rounded-lg px-2.5 text-left text-[13px] text-[var(--text)] transition-colors hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)]"
            >
              Edit address
              <kbd className="ml-auto text-[10px] text-[var(--quiet)]">N</kbd>
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

/**
 * The mark that means "leaves this app".
 *
 * Drawn rather than imported because Lucide's `ExternalLink` carries a box-and-
 * arrow that reads as "another window", and this opens the *system* browser --
 * a different program entirely. Three strokes say that without a tooltip.
 */
function ExternalMark() {
  return (
    <svg width="14" height="14" viewBox="0 0 14 14" fill="none" aria-hidden className="shrink-0">
      <path d="M4.5 9.5 9.5 4.5" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" />
      <path d="M6 4.5h3.5V8" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round" />
      <path d="M10 8.5v3.5H2.5V4.5H6" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}