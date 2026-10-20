import { useEffect, useMemo, useRef, useState } from "react";
import { Search } from "lucide-react";
import type { SettingsEntry } from "./settingsIndex";
import type { SettingsPage } from "./settingsPages";

/** How many matches the dropdown draws. More than this is a list, not a suggestion. */
const MAX_RESULTS = 8;

/**
 * The page a result lives on.
 *
 * Kept beside the entries rather than in the modal because the mapping belongs to the
 * index: an entry names its page by view, and any caller that navigates needs it. Every
 * view but one is complete as `{ view }` -- see `SettingsView` -- and the provider form is
 * the exception: choosing "Add provider" means the blank form, so it is opened with no
 * endpoint to edit. That is the one place a record would otherwise have to be invented.
 */
export function pageForEntry(entry: SettingsEntry): SettingsPage {
  if (entry.view === "form") return { view: "form", endpoint: null };
  return { view: entry.view };
}

/**
 * Scores one entry against a query, or `null` for no match.
 *
 * **Prefix beats contains, label beats keywords, page name matters least.** These are short
 * lists and a hallucinated ranking is worse than a simple one: a user typing "theme" wants
 * Theme, not Browser profile; one typing "browser" wants the browser rows and not the
 * page called Appearance. Ordering by where the match landed is enough to get that right,
 * and it is explainable, which a score nobody can predict never is.
 */
function score(entry: SettingsEntry, needle: string): number | null {
  const label = entry.label.toLowerCase();
  if (label.startsWith(needle)) return 0;
  if (label.includes(needle)) return 1;
  if (entry.keywords.some((word) => word.startsWith(needle))) return 2;
  if (entry.keywords.some((word) => word.includes(needle))) return 3;
  if (entry.page.toLowerCase().includes(needle)) return 4;
  return null;
}

/**
 * Search across every settings row, in the sidebar.
 *
 * The window has three groups and eleven sections, which is past the point where a user
 * can remember which page owns a row -- "where do I turn off completion notifications"
 * has one answer and eleven places it might be. Search answers both halves: *which* page
 * (the result says so) and *where* on it (the row is marked once the page opens).
 *
 * **Keyboard first.** Arrow keys move, Enter opens, Escape closes -- the field is where a
 * keyboard user already is, and forcing a trip to the pointer to pick a result undoes the
 * point of having typed a query. The list is `role="listbox"` and the active row is
 * reported through `aria-activedescendant`, so a screen reader follows along.
 */
export default function SettingsSearch({
  entries,
  onOpen,
}: {
  entries: SettingsEntry[];
  onOpen: (entry: SettingsEntry) => void;
}) {
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const [open, setOpen] = useState(false);
  const fieldRef = useRef<HTMLInputElement>(null);
  const rootRef = useRef<HTMLDivElement>(null);

  // Recomputed from the entries rather than stored: the index is a constant list and a
  // query is one string, so a filtered copy kept in state would only be a second copy of
  // an answer that is cheaper to derive than to keep in step.
  const matches = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (needle.length === 0) return [];
    return entries
      .map((entry) => ({ entry, rank: score(entry, needle) }))
      .filter((row): row is { entry: SettingsEntry; rank: number } => row.rank !== null)
      .sort((left, right) => left.rank - right.rank)
      .slice(0, MAX_RESULTS)
      .map((row) => row.entry);
  }, [entries, query]);

  const resultsOpen = open && query.trim().length > 0;
  const activeIndex = matches.length > 0 ? Math.min(active, matches.length - 1) : 0;

  // A click anywhere else closes the list. `pointerdown` rather than `click`, because the
  // click that lands on a page's own control would otherwise arrive with the list still
  // over it -- and a result chosen by pressing the background is never what was meant.
  useEffect(() => {
    if (!resultsOpen) return;
    const onPointerDown = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", onPointerDown);
    return () => document.removeEventListener("pointerdown", onPointerDown);
  }, [resultsOpen]);

  const choose = (entry: SettingsEntry) => {
    onOpen(entry);
    setOpen(false);
    setQuery("");
    setActive(0);
    // Focus stays in the window, on the page it just opened, rather than on a field that
    // would reopen the list as soon as anything was typed.
    fieldRef.current?.blur();
  };

  return (
    <div ref={rootRef} className="relative min-w-0 flex-1 sm:max-w-[320px]">
      <label className="flex min-h-8 items-center gap-2 rounded-md border border-transparent bg-[var(--raised)] px-2.5 transition-colors focus-within:border-[var(--accent)] focus-within:bg-[var(--page)]">
        <Search size={14} className="shrink-0 text-[var(--quiet)]" aria-hidden="true" />
        <input
          ref={fieldRef}
          type="search"
          role="combobox"
          aria-expanded={resultsOpen}
          aria-controls="settings-search-results"
          aria-autocomplete="list"
          aria-activedescendant={resultsOpen && matches.length > 0 ? `settings-search-${activeIndex}` : undefined}
          aria-label="Search settings"
          placeholder="Search settings"
          value={query}
          onChange={(event) => {
            setQuery(event.target.value);
            setActive(0);
            setOpen(true);
          }}
          onFocus={() => setOpen(true)}
          onKeyDown={(event) => {
            if (event.key === "ArrowDown") {
              event.preventDefault();
              setOpen(true);
              setActive((index) => (matches.length === 0 ? 0 : (index + 1) % matches.length));
            } else if (event.key === "ArrowUp") {
              event.preventDefault();
              setActive((index) => (matches.length === 0 ? 0 : (index - 1 + matches.length) % matches.length));
            } else if (event.key === "Enter") {
              const chosen = matches[activeIndex];
              if (resultsOpen && chosen) {
                event.preventDefault();
                choose(chosen);
              }
            } else if (event.key === "Escape") {
              // Clears before closing: Escape in a search field is normally "empty it",
              // and a second press then closes a list that is already gone. Not stopped,
              // though -- letting Escape through is what still closes the dialog when the
              // field is empty and no list is up.
              event.preventDefault();
              if (query.length > 0) setQuery("");
              else setOpen(false);
            }
          }}
          className="min-w-0 flex-1 bg-transparent text-[12px] text-[var(--text)] outline-none placeholder:text-[var(--quiet)] [&::-webkit-search-cancel-button]:hidden"
        />
      </label>

      {resultsOpen && (
        <div
          id="settings-search-results"
          role="listbox"
          aria-label="Settings results"
          className="absolute left-0 right-0 top-full z-50 mt-1.5 max-h-[min(360px,50vh)] overflow-y-auto rounded-lg border border-[var(--line)] bg-[var(--panel)] p-1 shadow-2xl"
        >
          {matches.length === 0 ? (
            <p className="px-2.5 py-2 text-[12px] text-[var(--quiet)]">No setting matches that.</p>
          ) : (
            matches.map((entry, index) => (
              <button
                key={entry.anchor}
                id={`settings-search-${index}`}
                type="button"
                role="option"
                aria-selected={index === activeIndex}
                // Mouse-down, like the slash menu: the field holds focus, and a click that
                // moved focus first would blur -- and close the list -- before the choice
                // was reported.
                onMouseDown={(event) => {
                  event.preventDefault();
                  choose(entry);
                }}
                onMouseEnter={() => setActive(index)}
                className={`flex w-full items-center gap-2 rounded px-2.5 py-1.5 text-left focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)] ${
                  index === activeIndex ? "bg-[var(--raised)]" : ""
                }`}
              >
                <span className="min-w-0 flex-1 truncate text-[12px] text-[var(--text)]">{entry.label}</span>
                <span className="shrink-0 text-[10px] uppercase tracking-wide text-[var(--quiet)]">{entry.page}</span>
              </button>
            ))
          )}
        </div>
      )}
    </div>
  );
}
