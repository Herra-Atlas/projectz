import { CircleHelp } from "lucide-react";

/**
 * A `?` beside a label, whose answer is the tooltip.
 *
 * The same affordance the job editor uses, and for the same reason: with a form
 * of any length, a paragraph under every field buries the fields it is
 * describing, and the explanation is only wanted when it is asked for. A `title`
 * rather than a custom popover, because the answer is a sentence rather than a
 * panel -- and because a hover card that has to be positioned is a second thing
 * to go wrong on a surface this small.
 */
export default function HelpMark({ hint }: { hint: string }) {
  return (
    <span
      title={hint}
      className="cursor-help text-[var(--quiet)] transition-colors hover:text-[var(--text)]"
    >
      <CircleHelp size={13} aria-label={hint} />
    </span>
  );
}
