import { useEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { ChevronDown, ChevronUp, Search, SlidersHorizontal, X } from "lucide-react";
import { useNotificationHost } from "../../features/notifications/notificationHost";
import { FIND_MATCH_CAP, useFind } from "../../features/find/useFind";
import FindOptionsPanel from "./FindOptionsPanel";

/**
 * The app's find bar.
 *
 * **It replaces the webview's, and it has to be an overlay.** The browser's find
 * bar could not be styled, could not dim the page and could not be told what a
 * page even is here -- it searches the whole document, sidebar and composer
 * included. This one searches the surfaces that declared themselves scopes
 * (`data-find-scope`) and fades everything in them but the matches.
 *
 * **Mounted once, in `App`, and always present.** The bar animates open and shut
 * rather than mounting and unmounting, so closing it is a transition instead of a
 * pop, and the state behind the search (`useFind`) survives every close.
 *
 * **Portalled into the settings dialog when one is open.** A `showModal()` dialog
 * is in the top layer, above every `z-index` on the page; a child of the dialog
 * is in that layer with it. The notification stack solved this the same way, and
 * the host it publishes is reused here rather than claimed a second time --
 * `useNotificationHost` is only a read of where the top layer currently is.
 */
export default function FindBar() {
  const find = useFind();
  const host = useNotificationHost();
  const fieldRef = useRef<HTMLInputElement>(null);
  const [optionsOpen, setOptionsOpen] = useState(false);
  /**
   * Where the bar hangs while it is portaled into a dialog.
   *
   * `position: fixed` inside a `showModal()` dialog is fixed to the *window*, and
   * the settings panel is a centred column narrower than the window -- so the
   * default corner would put the bar on the backdrop, unattached to the page it
   * searches. Measuring the panel's own box puts it inside, just under the
   * dialog's header, which is the same measured-popover rule the resident-server
   * menu follows. `null` means no dialog: the stylesheet's corner is used.
   */
  const [anchor, setAnchor] = useState<{ top: number; right: number } | null>(null);

  useEffect(() => {
    if (!host || !find.open) {
      setAnchor(null);
      return;
    }
    const panel = (host.firstElementChild as HTMLElement | null) ?? host;
    const measure = () => {
      const box = panel.getBoundingClientRect();
      setAnchor({
        // Clears the dialog's 60px header rather than covering its own search.
        top: Math.round(box.top + 72),
        right: Math.round(window.innerWidth - box.right + 16),
      });
    };
    measure();
    window.addEventListener("resize", measure);
    return () => window.removeEventListener("resize", measure);
  }, [host, find.open]);

  // Focus and select on every Ctrl+F, including one pressed while the bar is
  // already open -- the second press is a request for the field, not a no-op.
  useEffect(() => {
    if (!find.focusTick) return;
    const field = fieldRef.current;
    if (!field) return;
    field.focus();
    field.select();
  }, [find.focusTick]);

  // A closed bar has no open drawer; otherwise reopening would show whatever was
  // left expanded by the last search.
  useEffect(() => {
    if (!find.open) setOptionsOpen(false);
  }, [find.open]);

  const close = () => {
    setOptionsOpen(false);
    find.close();
  };

  const count = find.total > 0
    ? `${find.active + 1}/${find.total}${find.truncated ? "+" : ""}`
    : find.query ? "No results" : "";

  // A switch changed from its default is a search that behaves differently from
  // what the field alone suggests, so the icon keeps a mark instead of the panel
  // having to be opened to find out why a match is missing.
  const tuned = find.options.caseSensitive || find.options.wholeWord || !find.options.ignoreAccents;

  const bar = (
    <div
      // `data-find-ui` marks the bar's own DOM, which is also what keeps it out of
      // any scope that happens to contain the portal target.
      data-find-ui=""
      data-open={find.open}
      style={anchor ?? undefined}
      className="pz-find fixed right-4 top-[68px] z-[125] w-[min(430px,calc(100vw-2rem))] rounded-xl border border-[var(--line-strong)] bg-[color-mix(in_srgb,var(--panel)_94%,transparent)] shadow-[0_18px_44px_-24px_rgba(0,0,0,0.65)] backdrop-blur-md"
      role="search"
      aria-label="Find in this view"
      inert={!find.open}
      aria-hidden={!find.open}
      onKeyDown={(event) => {
        // Escape belongs to the bar only while the caret is in it. The chat page
        // uses Escape to stop speech, and a global handler here would end a reply
        // being read aloud because a search was left open.
        if (event.key === "Escape") {
          event.preventDefault();
          event.stopPropagation();
          close();
        }
      }}
    >
      <div className="flex items-center gap-1 py-1.5 pl-2.5 pr-1.5">
        <Search size={14} className="shrink-0 text-[var(--quiet)]" />
        <input
          ref={fieldRef}
          type="text"
          value={find.query}
          onChange={(event) => find.setQuery(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              find.step(event.shiftKey ? -1 : 1);
            }
          }}
          aria-label={`Find in ${find.label}`}
          placeholder={`Find in ${find.label}`}
          spellCheck={false}
          autoComplete="off"
          autoCorrect="off"
          className="h-8 min-w-0 flex-1 bg-transparent px-1.5 text-[13px] text-[var(--text)] placeholder:text-[var(--quiet)]"
        />
        {/* Keyed so the figure re-animates when it moves: a count that silently
            changes is easy to miss on a page that also moved. */}
        <span
          key={`${find.active}:${find.total}`}
          aria-live="polite"
          // A `title` only when there is something to explain; the app's tooltip
          // layer turns it into the same card every other control gets.
          title={find.truncated ? `Only the first ${FIND_MATCH_CAP} matches are listed` : undefined}
          className={`pz-find-tick shrink-0 pl-1 text-[11px] tabular-nums ${find.total === 0 && find.query ? "text-[var(--quiet)]" : "text-[var(--muted)]"}`}
        >
          {count}
        </span>
        <span aria-hidden="true" className="mx-0.5 h-5 w-px shrink-0 bg-[var(--line)]" />
        <BarButton label="Previous match" hint="Previous match · Shift+Enter" disabled={find.total === 0} onClick={() => find.step(-1)}>
          <ChevronUp size={15} />
        </BarButton>
        <BarButton label="Next match" hint="Next match · Enter" disabled={find.total === 0} onClick={() => find.step(1)}>
          <ChevronDown size={15} />
        </BarButton>
        <BarButton
          label="Search options"
          hint="Match case, whole word, accents"
          pressed={optionsOpen}
          onClick={() => setOptionsOpen((open) => !open)}
          className={tuned && !optionsOpen ? "text-[var(--accent)]" : undefined}
        >
          <SlidersHorizontal size={14} />
          {tuned && !optionsOpen && (
            <span aria-hidden="true" className="absolute right-1 top-1 size-1 rounded-full bg-[var(--accent)]" />
          )}
        </BarButton>
        <BarButton label="Close find" hint="Close find · Esc" onClick={close}>
          <X size={15} />
        </BarButton>
      </div>
      <FindOptionsPanel open={optionsOpen} options={find.options} onChange={find.setOptions} />
    </div>
  );

  return createPortal(bar, host ?? document.body);
}

type BarButtonProps = {
  label: string;
  /** What the hover card says. Defaults to the accessible name. */
  hint?: string;
  onClick: () => void;
  disabled?: boolean;
  /** Rendered as a toggle rather than a plain button, for the options drawer. */
  pressed?: boolean;
  className?: string;
  children: ReactNode;
};

function BarButton({ label, hint, onClick, disabled, pressed, className = "", children }: BarButtonProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      // Short for assistive technology, longer for the pointer: `title` is what
      // `TooltipLayer` lifts into the app's own card, and it is the one place
      // these shortcuts are written down.
      aria-label={label}
      title={hint ?? label}
      aria-pressed={pressed}
      className={`relative grid size-7 shrink-0 place-items-center rounded-md text-[var(--muted)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)] disabled:opacity-30 disabled:hover:bg-transparent disabled:hover:text-[var(--muted)] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)] ${pressed ? "bg-[var(--raised)] text-[var(--text)]" : ""} ${className}`}
    >
      {children}
    </button>
  );
}
