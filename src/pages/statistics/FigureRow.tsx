import { useMemo, useState } from "react";
import { ArrowUpDown, Brain, ChevronLeft, ChevronRight, Coins, Globe2, MessageSquare, MessageSquareText, Zap } from "lucide-react";
import { formatTokens } from "../../features/chat/sessionStats";
import type { UsageReport } from "../../features/statistics/types";
import { FigureCard } from "./FigureCard";

/** Figures shown four at a time, with a control to page through the rest.
 *
 * All eight are kept on one object rather than spread across two grids so a new
 * figure is one line here and the paging handles the layout. `em dash` stands in
 * for a figure that has not loaded yet. */
const FIGURES_PER_PAGE = 4;

type Figure = { icon: React.ReactNode; label: string; value: string };

/** Turns a report into the flat list of figures shown in the header. */
const figuresFor = (report: UsageReport | null): Figure[] => {
  if (!report) {
    return [
      { icon: <MessageSquareText size={15} />, label: "Conversations", value: "—" },
      { icon: <Coins size={15} />, label: "Total tokens", value: "—" },
      { icon: <ArrowUpDown size={15} />, label: "Input tokens", value: "—" },
      { icon: <ArrowUpDown size={15} />, label: "Output tokens", value: "—" },
    ];
  }
  return [
    { icon: <MessageSquareText size={15} />, label: "Conversations", value: report.sessions.toLocaleString() },
    { icon: <Coins size={15} />, label: "Total tokens", value: formatTokens(report.total_tokens) },
    { icon: <ArrowUpDown size={15} />, label: "Input tokens", value: formatTokens(report.prompt_tokens) },
    { icon: <ArrowUpDown size={15} />, label: "Output tokens", value: formatTokens(report.completion_tokens) },
    { icon: <MessageSquare size={15} />, label: "Messages sent", value: report.user_messages.toLocaleString() },
    { icon: <MessageSquareText size={15} />, label: "Replies", value: report.assistant_messages.toLocaleString() },
    { icon: <Globe2 size={15} />, label: "Web searches", value: report.searches.toLocaleString() },
    { icon: <Brain size={15} />, label: "Models used", value: report.models.length.toLocaleString() },
    { icon: <Zap size={15} />, label: "Cache hit rate", value: cacheHitRate(report) },
  ];
};

/**
 * The share of prompt tokens the provider served from its cache.
 *
 * Measured only over the replies that reported a figure at all, because a
 * provider that reports nothing is not a provider that cached nothing. So
 * `cache_prompt_tokens` is already narrowed to those replies by the query and
 * dividing by the period's whole `prompt_tokens` here would dilute the rate with
 * traffic that never had a chance to hit.
 *
 * `—` when nothing reported a cache figure. A `0%` would claim every reply
 * missed the cache, which is a claim about a provider that never answered the
 * question.
 */
function cacheHitRate(report: UsageReport): string {
  if (report.cache_reported_replies === 0 || report.cache_prompt_tokens <= 0) return "—";
  const rate = report.cached_tokens / report.cache_prompt_tokens;
  return `${(rate * 100).toFixed(rate >= 0.1 ? 0 : 1)}%`;
}

/** The paged figure row. The control only appears when there is more than one
 * page, so a report with four or fewer figures shows no arrow at all. */
export default function FigureRow({ report }: { report: UsageReport | null }) {
  const [page, setPage] = useState(0);
  const figures = useMemo(() => figuresFor(report), [report]);
  const pageCount = Math.max(1, Math.ceil(figures.length / FIGURES_PER_PAGE));
  // A timeframe switch can leave the page past the end, so clamp rather than
  // render an empty grid.
  const current = Math.min(page, pageCount - 1);
  const visible = figures.slice(current * FIGURES_PER_PAGE, current * FIGURES_PER_PAGE + FIGURES_PER_PAGE);

  return (
    <div className="flex items-center gap-1.5">
      <div className="grid min-w-0 flex-1 grid-cols-1 gap-2 sm:grid-cols-2 lg:grid-cols-4">
        {visible.map((figure) => <FigureCard key={figure.label} {...figure} />)}
      </div>
      {/* The arrows sit close to the grid and stop at the ends rather than
          wrapping, so the row reads as a position in a list instead of a
          carousel that never ends. Each arrow is disabled on the page it would
          not move from. */}
      {pageCount > 1 && (
        /* The arrows are centred against the card row. The page count is taken
           out of the flow and hung below them, so it cannot push the pair
           upwards out of the middle. */
        <div className="relative flex shrink-0 self-stretch items-center">
          <div className="flex flex-col items-center">
            <button
              type="button"
              onClick={() => setPage((value) => Math.max(0, value - 1))}
              className="grid size-6 place-items-center rounded text-[var(--quiet)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-[var(--accent)] disabled:pointer-events-none disabled:opacity-25"
              aria-label="Previous figures"
              disabled={current === 0}
            >
              <ChevronLeft size={14} />
            </button>
            <button
              type="button"
              onClick={() => setPage((value) => Math.min(pageCount - 1, value + 1))}
              className="grid size-6 place-items-center rounded text-[var(--quiet)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-[var(--accent)] disabled:pointer-events-none disabled:opacity-25"
              aria-label="Next figures"
              disabled={current === pageCount - 1}
            >
              <ChevronRight size={14} />
            </button>
          </div>
          <span
            className="absolute left-1/2 top-full -translate-x-1/2 pt-0.5 text-[10px] tabular-nums text-[var(--quiet)]"
            aria-live="polite"
          >
            {current + 1}/{pageCount}
          </span>
        </div>
      )}
    </div>
  );
}
