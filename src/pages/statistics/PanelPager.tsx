import { ChevronLeft, ChevronRight } from "lucide-react";

/**
 * Arrows for viewing the next view inside a panel, matching `FigureRow`'s paging.
 *
 * The count is the same shape as the card row's -- a position out of how many --
 * so the two controls read as one system rather than two. It sits inline rather
 * than beneath the arrows, because a panel header has height to spare where the
 * card row has none.
 *
 * The label names the view being shown, so the control says what the arrows do
 * rather than only that there are two of them. On a single-view panel nothing is
 * drawn at all: arrows that go nowhere are a control that lies about being one.
 */
type PanelPagerProps = {
  /** Total number of views, one more than the highest index. */
  count: number;
  /** Zero-based index of the view on screen. */
  index: number;
  /** Names of the views, so the position reads as a name rather than a count. */
  labels: string[];
  onChange: (index: number) => void;
  /** Accessible description of what is being paged, e.g. "chart". */
  what: string;
};

export function PanelPager({ count, index, labels, onChange, what }: PanelPagerProps) {
  if (count <= 1) return null;
  return (
    <div className="flex shrink-0 items-center gap-0.5">
      <button
        type="button"
        onClick={() => onChange(index - 1)}
        disabled={index === 0}
        className="grid size-5 place-items-center rounded text-[var(--quiet)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-[var(--accent)] disabled:pointer-events-none disabled:opacity-25"
        aria-label={`Previous ${what}`}
      >
        <ChevronLeft size={13} />
      </button>
      <span className="min-w-5 text-center text-[10px] tabular-nums text-[var(--quiet)]" aria-live="polite">
        {index + 1}/{count}
      </span>
      <button
        type="button"
        onClick={() => onChange(index + 1)}
        disabled={index === count - 1}
        className="grid size-5 place-items-center rounded text-[var(--quiet)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-[var(--accent)] disabled:pointer-events-none disabled:opacity-25"
        aria-label={`Next ${what}`}
      >
        <ChevronRight size={13} />
      </button>
      <span className="sr-only">{labels[index]}</span>
    </div>
  );
}