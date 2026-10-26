import { CircleHelp } from "lucide-react";

/**
 * A `?` beside a label.
 *
 * The hint is a plain `title`, not a hand-built popover, because `TooltipLayer`
 * owns the drawing: it borrows the attribute on hover or focus and paints the
 * themed card, then puts the title back. That is what makes a `?` look like
 * every other hover in the app instead of a second tooltip implementation.
 *
 * `tabIndex` is here because a `?` is an answer waiting to be asked: the layer
 * treats `focusin` the same as `pointerover`, so the keyboard gets the hint too.
 */
export default function HelpMark({ hint }: { hint: string }) {
  return (
    <span
      title={hint}
      tabIndex={0}
      className="inline-flex cursor-help items-center rounded-full text-[var(--quiet)] outline-none transition-colors hover:text-[var(--text)] focus-visible:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
    >
      <CircleHelp size={13} aria-label={hint} />
    </span>
  );
}
