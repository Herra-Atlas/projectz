import { useEffect, useRef, useState } from "react";
import { Check, SlidersHorizontal } from "lucide-react";

/** One section of a kind's catalogue, counted from the rows on screen. */
export type DownloadSection = { label: string; count: number };

/**
 * The section filter beside a download search.
 *
 * It filters what has already been fetched rather than asking again: the search
 * request is what costs a round trip (thirty of them, for llama.cpp), and a
 * section is a property of the answer, not of the question. So the list narrows
 * instantly and the counts stay honest for the search that produced them.
 *
 * Multi-select, because a reader looking for either a voice or a recogniser has
 * no reason to be forced to choose one; nothing selected means everything, which
 * is why "All sections" is a clear rather than a fourth checkbox.
 */
export function DownloadGroups({
  sections,
  selected,
  onToggle,
  onClear,
}: {
  sections: DownloadSection[];
  selected: ReadonlySet<string>;
  onToggle: (label: string) => void;
  onClear: () => void;
}) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);

  // Dismissed on a press anywhere else, and on Escape. The panel is anchored to
  // the button rather than portalled, so `contains` is enough to tell a press
  // inside it from one that should close it.
  useEffect(() => {
    if (!open) return;
    const dismiss = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      setOpen(false);
      trigger.current?.focus();
    };
    document.addEventListener("pointerdown", dismiss);
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("pointerdown", dismiss);
      document.removeEventListener("keydown", escape);
    };
  }, [open]);

  const chosen = Array.from(selected);
  const summary =
    chosen.length === 0 ? "All" : chosen.length === 1 ? chosen[0] : `${chosen.length} sections`;

  return (
    <div ref={root} className="relative shrink-0">
      <button
        ref={trigger}
        type="button"
        onClick={() => setOpen((current) => !current)}
        aria-expanded={open}
        aria-controls="download-sections"
        aria-label={`Filter by section: ${summary}`}
        title="Filter by section"
        className={`inline-flex min-h-9 items-center gap-1.5 rounded-md border px-2.5 text-[12px] transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)] ${chosen.length > 0 ? "border-[color-mix(in_srgb,var(--accent)_45%,var(--line))] text-[var(--accent)]" : "border-[var(--line)] text-[var(--muted)] hover:border-[var(--line-strong)] hover:text-[var(--text)]"}`}
      >
        <SlidersHorizontal size={13} />
        <span className="max-w-[14ch] truncate">{summary}</span>
      </button>

      {open && (
        <div
          id="download-sections"
          role="group"
          aria-label="Filter by section"
          className="absolute right-0 top-full z-20 mt-1 min-w-[196px] rounded-lg border border-[var(--line)] bg-[var(--rail)] p-1 shadow-2xl"
        >
          <button
            type="button"
            onClick={onClear}
            aria-pressed={chosen.length === 0}
            className="flex min-h-8 w-full items-center gap-2.5 rounded-md px-2 text-left text-xs text-[var(--text)] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)]"
          >
            <span className="grid size-3.5 shrink-0 place-items-center text-[var(--accent)]">
              {chosen.length === 0 && <Check size={12} />}
            </span>
            <span className="flex-1">All sections</span>
          </button>
          <div className="my-1 border-t border-[var(--line)]" />
          {sections.map((section) => {
            const on = selected.has(section.label);
            return (
              <button
                key={section.label}
                type="button"
                onClick={() => onToggle(section.label)}
                aria-pressed={on}
                className="flex min-h-8 w-full items-center gap-2.5 rounded-md px-2 text-left text-xs text-[var(--text)] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)]"
              >
                <span className="grid size-3.5 shrink-0 place-items-center text-[var(--accent)]">
                  {on && <Check size={12} />}
                </span>
                <span className="flex-1 truncate">{section.label}</span>
                <span className="shrink-0 text-[11px] tabular-nums text-[var(--quiet)]">
                  {section.count}
                </span>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
