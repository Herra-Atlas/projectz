import { useMemo, useState } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { formatTokens } from "../../features/chat/sessionStats";
import { formatBucket, type AxisGranularity } from "../../features/statistics/timeframes";
import type { UsageReport } from "../../features/statistics/types";
import { FigureCard, type FigureChange } from "./FigureCard";

/** Figures shown four at a time, with a control to page through the rest.
 *
 * All of them are kept on one list rather than spread across two grids so a new
 * figure is one line here and the paging handles the layout. `em dash` stands in
 * for a figure that has not loaded yet. */
const FIGURES_PER_PAGE = 4;

type Figure = {
  label: string;
  value: string;
  /** The move against the previous period, or null when there is nothing to
      compare against. */
  change: FigureChange | null;
  /** What the comparison is against — `vs 12.3M prior` — or why there is none. */
  note: string;
};

/** A change, with one decimal only where a whole number would lose the move:
    16% is a reading, 2.4% is the whole message. */
const formatPercent = (value: number) => (value >= 10 ? value.toFixed(0) : value.toFixed(1));

/** Turns a signed change into a pill. `suffix` is `%` for a ratio or ` pts` for
    a difference between two rates, which is a different claim from a ratio. */
const changeOf = (change: number, suffix: string): FigureChange => {
  const direction = Math.abs(change) < 0.05 ? "flat" : change > 0 ? "up" : "down";
  return { direction, label: direction === "flat" ? `0${suffix}` : `${formatPercent(Math.abs(change))}${suffix}` };
};

/**
 * A count or token figure compared with the same figure a period earlier.
 *
 * No previous usage is not a rise of 100%: it is the absence of a comparison,
 * so the card says so in words and draws no pill. Claiming a percentage against
 * nothing is the one reading a change badge must never give.
 */
const compare = (
  current: number,
  previous: number,
  format: (value: number) => string,
): Pick<Figure, "change" | "note"> =>
  previous > 0
    ? { change: changeOf(((current - previous) / previous) * 100, "%"), note: `vs ${format(previous)} prior` }
    : { change: null, note: "none in prior period" };

/** The share of prompt tokens a provider served from its cache, or null when no
    reply reported a cache figure at all. Unknown is not zero. */
const cacheRate = (cached: number, prompt: number, reported: number): number | null =>
  reported > 0 && prompt > 0 ? cached / prompt : null;

const formatRate = (rate: number | null) =>
  rate === null ? "—" : `${(rate * 100).toFixed(rate >= 0.1 ? 0 : 1)}%`;

/** The buckets that cost the most, and when the biggest one was.
 *
 * A peak is the per-bucket total rather than a per-reply one: the chart above
 * draws the same numbers, so the card is a reading of the chart's highest point
 * rather than a second measurement that could disagree with it. `null` on a
 * period with nothing plotted. */
const peakFor = (timeline: UsageReport["timeline"]) =>
  timeline.reduce<{ bucket: string; total: number } | null>((best, entry) => {
    const total = entry.prompt_tokens + entry.completion_tokens;
    return !best || total > best.total ? { bucket: entry.bucket, total } : best;
  }, null);

/**
 * Turns a report into the flat list of figures shown in the header.
 *
 * The order is the order the numbers are read in: spend first, then what made
 * it, then the counts, then the two derived measures. "All" has no period before
 * it, so it draws no comparisons rather than claiming nothing happened before.
 */
const figuresFor = (report: UsageReport | null, axis: AxisGranularity): Figure[] => {
  if (!report) {
    return ["Total tokens", "Input tokens", "Output tokens", "Requests"].map((label) => ({
      label,
      value: "—",
      change: null,
      note: "",
    }));
  }

  const previous = report.previous;
  const count = (value: number) => value.toLocaleString();
  const comparable = report.range !== "all";
  const against = (current: number, prior: number, format: (value: number) => string): Pick<Figure, "change" | "note"> =>
    comparable ? compare(current, prior, format) : { change: null, note: "" };
  const currentRate = cacheRate(report.cached_tokens, report.cache_prompt_tokens, report.cache_reported_replies);
  const previousRate = cacheRate(previous.cached_tokens, previous.cache_prompt_tokens, previous.cache_reported_replies);
  const peak = peakFor(report.timeline);

  return [
    { label: "Total tokens", value: formatTokens(report.total_tokens), ...against(report.total_tokens, previous.total_tokens, formatTokens) },
    { label: "Input tokens", value: formatTokens(report.prompt_tokens), ...against(report.prompt_tokens, previous.prompt_tokens, formatTokens) },
    { label: "Output tokens", value: formatTokens(report.completion_tokens), ...against(report.completion_tokens, previous.completion_tokens, formatTokens) },
    // A reply that ran tools made several requests, so this is the count of calls
    // rather than of answers -- the distinction the old Replies figure could not
    // make.
    { label: "Requests", value: count(report.requests), ...against(report.requests, previous.requests, count) },
    // Taste is its own feature with its own run table, so it gets its own figure
    // rather than disappearing into the conversation total.
    { label: "Taste tokens", value: formatTokens(report.taste_total_tokens), ...against(report.taste_total_tokens, previous.taste_total_tokens, formatTokens) },
    { label: "Conversations", value: count(report.sessions), ...against(report.sessions, previous.sessions, count) },
    { label: "Messages sent", value: count(report.user_messages), ...against(report.user_messages, previous.user_messages, count) },
    { label: "Models used", value: count(report.models.length), ...against(report.models.length, previous.models, count) },
    {
      label: "Cache hit rate",
      value: formatRate(currentRate),
      // Two rates are compared in percentage points, not in percent: going from
      // 20% to 24% is four points, and calling it a 20% rise would describe a
      // different quantity.
      change: comparable && currentRate !== null && previousRate !== null
        ? changeOf((currentRate - previousRate) * 100, " pts")
        : null,
      note: !comparable
        ? ""
        : previousRate !== null ? `vs ${formatRate(previousRate)} prior` : "no prior figure",
    },
    {
      label: "Peak tokens",
      value: peak ? formatTokens(peak.total) : "—",
      change: null,
      // The bucket the peak fell in, in the same words the chart's axis uses.
      note: peak ? formatBucket(peak.bucket, axis) : "nothing in this period",
    },
  ];
};

/** The paged figure row.
 *
 * The cards share one bordered block with hairline gaps rather than each
 * carrying its own border: the dividers belong to the row, so the figures read
 * as one strip of measurements instead of four competing panels. The paging
 * control only appears when there is more than one page.
 */
export default function FigureRow({ report, axis }: { report: UsageReport | null; axis: AxisGranularity }) {
  const [page, setPage] = useState(0);
  const figures = useMemo(() => figuresFor(report, axis), [report, axis]);
  const pageCount = Math.max(1, Math.ceil(figures.length / FIGURES_PER_PAGE));
  // A timeframe switch can leave the page past the end, so clamp rather than
  // render an empty grid.
  const current = Math.min(page, pageCount - 1);
  const visible = figures.slice(current * FIGURES_PER_PAGE, (current + 1) * FIGURES_PER_PAGE);
  // A last page of two needs two blank cells in the four-wide row: the gaps are
  // the divider colour, so a missing cell would show as a block rather than as a
  // hairline. They are hidden below `lg`, where two cards are a whole row and an
  // empty cell would only add a hairline beneath them.
  const cells: (Figure | null)[] = [
    ...visible,
    ...Array.from({ length: FIGURES_PER_PAGE - visible.length }, () => null),
  ];

  return (
    <div className="flex items-center gap-1.5">
      <div className="grid min-w-0 flex-1 grid-cols-2 gap-px overflow-hidden rounded-lg border border-[var(--line)] bg-[var(--line)] lg:grid-cols-4">
        {cells.map((figure, index) =>
          figure
            ? <FigureCard key={figure.label} {...figure} />
            : <div key={`empty-${index}`} className="hidden bg-[var(--panel)] lg:block" />,
        )}
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
