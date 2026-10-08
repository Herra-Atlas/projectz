/**
 * The composer's Effort control.
 *
 * Split out because it is the one piece of the composer that owns its own open
 * state and its own dismiss behaviour, and both of those are about *this* menu:
 * a click elsewhere or an Escape closing it, and nothing else on screen noticing.
 *
 * The levels are the selected model's own, sent verbatim. Translating them here
 * is what caused the HTTP 400s -- a provider rejects a level it did not list.
 */

import { useEffect, useRef, useState } from "react";
import { Check, ChevronDown } from "lucide-react";

/** Capitalises for display without imposing a fixed English vocabulary. */
const label = (value: string) => value.charAt(0).toUpperCase() + value.slice(1);

type EffortControlProps = {
  /** Whatever the provider listed for this model. Already empty when unsupported. */
  values: string[];
  value: string;
  onChange: (value: string) => void;
};

export default function EffortControl({ values, value, onChange }: EffortControlProps) {
  const [open, setOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const dismissOutside = (event: PointerEvent) => {
      if (containerRef.current && !containerRef.current.contains(event.target as Node)) setOpen(false);
    };
    const dismissEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("pointerdown", dismissOutside);
    document.addEventListener("keydown", dismissEscape);
    return () => {
      document.removeEventListener("pointerdown", dismissOutside);
      document.removeEventListener("keydown", dismissEscape);
    };
  }, [open]);

  return (
    <div ref={containerRef} className="relative">
      {/* "Effort" is dimmed so the label recedes; the value is not, because the
          current state is the information the user actually came to read. */}
      <button type="button" aria-expanded={open} aria-label={`Effort ${label(value)}`} onClick={() => setOpen((current) => !current)} className="inline-flex h-7 items-center gap-1 rounded-md px-2 text-[11px] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]">
        <span className="text-[var(--quiet)]">Effort</span>
        <span className={value === "none" ? "text-[var(--text)]" : "text-[var(--accent)]"}>{label(value)}</span>
        <ChevronDown size={11} className="text-[var(--quiet)]" />
      </button>
      {open && (
        <div className="absolute bottom-full right-0 z-40 mb-2 w-36 overflow-hidden rounded-md border border-[var(--line)] bg-[var(--rail)] p-1 shadow-xl">
          <p className="px-2 py-1.5 text-[10px] text-[var(--quiet)]">Effort</p>
          {values.map((level) => (
            <button key={level} type="button" onClick={() => { onChange(level); setOpen(false); }} aria-pressed={value === level} className="flex min-h-8 w-full items-center justify-between rounded px-2 text-left text-xs text-[var(--text)] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]">
              {label(level)}
              {value === level && <Check size={13} className="text-[var(--accent)]" />}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}