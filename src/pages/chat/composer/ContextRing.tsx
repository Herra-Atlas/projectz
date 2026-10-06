/**
 * The context-usage ring and its readout.
 *
 * Split out because it is the one composer control whose figure is *derived*
 * rather than stored -- `ChatPage` works out the numbers and this only draws
 * them, which keeps the arithmetic in one place instead of beside an SVG.
 *
 * The ring is deliberately not a hover surface: a background matching the
 * unfilled track made the icon vanish. Hovering lifts the track one step instead.
 */

import { useEffect, useRef, useState } from "react";

/** The ring's circumference at r=8.5, used to turn a percentage into a dash. */
const RING_CIRCUMFERENCE = 53.4;

export function compactTokenCount(count: number): string {
  if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1)}M`;
  if (count >= 10_000) return `${(count / 1_000).toFixed(1)}K`;
  return count.toLocaleString();
}

type ContextRingProps = {
  /** Tokens currently in the request. */
  tokens: number;
  /** The limit being measured against: a local model's setting, or the provider's. */
  limit: number;
  /**
   * Whether `tokens` is measured or inferred.
   *
   * Drawn as "Estimate" rather than a bare figure, because a character-count
   * guess presented like a provider's own token count is a claim the app cannot
   * support. It also suppresses the ring fill entirely when zero -- an empty
   * conversation still estimates tokens from the system prompt, and a lit sliver
   * implied usage that does not exist.
   */
  estimated: boolean;
  /** A local model uses its configured window, so the caveat differs. */
  isLocal: boolean;
};

export default function ContextRing({ tokens, limit, estimated, isLocal }: ContextRingProps) {
  const [open, setOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const usage = Math.min(100, (tokens / limit) * 100);

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
      <button type="button" onClick={() => setOpen((current) => !current)} aria-expanded={open} aria-label={`Context ${Math.round(usage)}% used`} className="group grid size-6 shrink-0 place-items-center rounded-full focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]" title="Context usage">
        <svg width="16" height="16" viewBox="0 0 22 22" className="-rotate-90" aria-hidden="true">
          <circle cx="11" cy="11" r="8.5" fill="none" strokeWidth="2.5" className="stroke-[var(--raised)] transition-colors group-hover:stroke-[var(--line)]" />
          {tokens > 0 && <circle cx="11" cy="11" r="8.5" fill="none" stroke="var(--accent)" strokeWidth="2.5" strokeLinecap="round" strokeDasharray={`${(usage / 100) * RING_CIRCUMFERENCE} ${RING_CIRCUMFERENCE}`} />}
        </svg>
      </button>
      {open && (
        <div className="absolute bottom-full right-0 z-40 mb-2 w-64 rounded-lg border border-[var(--line)] bg-[var(--panel)] p-3 shadow-xl">
          <div className="flex items-center justify-between text-xs">
            <span className="font-medium text-[var(--text)]">Context usage</span>
            <span className="text-[var(--muted)]">{estimated ? "Estimate" : "Measured"}</span>
          </div>
          <p className="mt-2 text-sm font-medium tabular-nums text-[var(--text)]">
            {compactTokenCount(tokens)} / {compactTokenCount(limit)} used ({Math.round(usage)}%)
          </p>
          <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-[var(--raised)]">
            <div className="h-full rounded-full bg-[var(--accent)] transition-[width]" style={{ width: `${usage}%` }} />
          </div>
          <p className="mt-2 text-[11px] leading-5 text-[var(--quiet)]">
            {estimated ? "Estimated from conversation text." : "From the provider's reported prompt tokens."}
            {isLocal ? " Limit is the configured local context window." : " Remote model limits may differ."}
          </p>
        </div>
      )}
    </div>
  );
}