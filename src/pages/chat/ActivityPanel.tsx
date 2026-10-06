import { useEffect, useMemo, useRef, useState } from "react";
import { Brain, ChevronRight, LoaderCircle } from "lucide-react";
import ToolIcon from "../../components/ToolIcon";
import type { ActivityStep, DiffLine, ThoughtStep, ToolDiff, ToolStep } from "../../features/chat/types";

/**
 * One row of the panel: either a single call, or a run of them folded together.
 *
 * Folding happens here rather than in the message shape because it needs the
 * whole list and no one else does. Reading three files in one turn is ordinary
 * and the useful fact about it is the count, not each path, so consecutive reads
 * share a row while anything else stands on its own.
 */
type Row =
  | { kind: "tool"; steps: ToolStep[] }
  | { kind: "thought"; step: ThoughtStep };

type ActivityPanelProps = {
  steps: ActivityStep[];
  /** True while the run these steps belong to is still going. */
  live: boolean;
  /** Markdown renderer for a thought's text. */
  renderMarkdown: (text: string) => React.ReactNode;
  /**
   * How long the whole reply took, when it is known.
   *
   * Preferred over summing the steps, because the sum is not the reply's
   * duration: the time spent streaming the answer belongs to no step, so a
   * reply that thought briefly and then typed for ten seconds sums to about
   * zero. "Worked for" is the span the user actually waited through, and the
   * caller measures that from send to completion -- which is also why it
   * includes time spent sitting on a permission prompt, since that is time
   * they spent waiting too.
   *
   * Omitted only by the live panel while it ticks, where there is no finished
   * figure yet and the elapsed clock is used instead.
   */
  totalSeconds?: number;
};

/**
 * The thinking and tool work behind one reply.
 *
 * Collapsed by default, because the answer is what the reader came for and a
 * panel open on every reply would bury it. The header states the total time and
 * the step count, so a collapsed panel is still readable — "Worked for 15s · 4
 * steps" answers the question without opening anything.
 *
 * **A run ticking while it happens.** The header counts up and each running row
 * counts up with it, so a reply that is stuck on a slow search is visibly stuck
 * rather than apparently idle. The clock is the caller's: `live` and the
 * `startedAt` on each running step are enough to derive it here, and a run that
 * has finished shows fixed figures from the backend instead.
 */
export default function ActivityPanel({ steps, live, renderMarkdown, totalSeconds }: ActivityPanelProps) {
  const [open, setOpen] = useState(false);
  // Open by default while the work is happening, so a slow run shows what it is
  // doing rather than a collapsed line counting up. Closed once it is done, which
  // is when the answer arrives and becomes the thing to read.
  const [wasLive, setWasLive] = useState(live);
  useEffect(() => {
    if (live) setOpen(true);
    if (wasLive && !live) setOpen(false);
    setWasLive(live);
  }, [live, wasLive]);

  const rows = useMemo(() => buildRows(steps), [steps]);
  const totals = usePanelTotals(steps, live, totalSeconds);

  if (steps.length === 0) return null;

  return (
    <section className="mb-3 max-w-2xl" aria-label="Reasoning and tools">
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        aria-expanded={open}
        className="flex min-h-8 w-fit items-center gap-1.5 rounded py-1 text-[12px] text-[var(--muted)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
      >
        <ChevronRight
          size={13}
          className={`shrink-0 transition-transform ${open ? "rotate-90" : ""}`}
        />
        <span className="text-[var(--muted)]">
          Worked for {formatSeconds(totals.seconds)}
        </span>
        {/* Tool uses only. A thought is a step the panel lists, but the count in
            the header is the number of things the agent *did* — the figure the
            user is scanning for. Counting a thought as one made a reply with a
            single read read as "2 steps", which is not what anyone means by
            that. Hidden entirely when there were no tools, so a reply that only
            reasoned does not claim it ran something. */}
        {totals.toolCount > 0 && (
          <span className="text-[var(--quiet)]">· {totals.toolCount} steps</span>
        )}
      </button>

      {open && (
        <ol className="mt-1 space-y-0.5 border-l border-[var(--line)] pl-3">
          {rows.map((row, index) =>
            row.kind === "thought" ? (
              <ThoughtRow key={`thought-${index}`} step={row.step} live={live} renderMarkdown={renderMarkdown} />
            ) : (
              <ToolRow key={`tool-${index}`} steps={row.steps} live={live} />
            ),
          )}
        </ol>
      )}
    </section>
  );
}

/**
 * One stretch of thinking.
 *
 * **Closed by default, like the panel itself.** A thought is the supporting
 * record of a reply, not the answer, and a reasoning trace can run to dozens of
 * lines: opened, it buries the tool calls that came after it and pushes the
 * answer itself off screen. The row label states the fact worth scanning for --
 * how long the model thought -- and the trace is one click away for anyone who
 * wants to audit how it got there.
 *
 * The same shape as the tool run's disclosure, so the panel reads as one set of
 * controls rather than two: a caret, a label, and content underneath.
 *
 * Rendered as markdown like the answer is, because a model writes its thinking
 * with the same code fences and lists it writes an answer with, and rendering
 * one as plain text while the other is formatted makes the thought look like
 * something the app did to it.
 */
function ThoughtRow({
  step,
  live,
  renderMarkdown,
}: {
  step: ThoughtStep;
  live: boolean;
  renderMarkdown: (text: string) => React.ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const now = useTick(live && step.running, step.startedAt);
  const seconds = stepSeconds(step, now, live);
  // A thought with nothing in it gets no disclosure, because a caret beside an
  // empty trace invites a click that reveals nothing.
  const collapsible = step.text.length > 0;
  // Immediately after the label, not pushed to a fixed column. The caret belongs
  // to the text it opens, and a reader tracks `Thought for 2s` across to its own
  // caret rather than to a position several rows away.
  const caret = (
    <ChevronRight
      size={12}
      aria-hidden
      className={`shrink-0 transition-transform ${open ? "rotate-90" : ""}`}
    />
  );
  return (
    <li className="text-[12px] leading-6 text-[var(--muted)]">
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        aria-expanded={collapsible ? open : undefined}
        // Not a button when there is nothing behind it: a caret that does not
        // open anything is a control that lies about being one.
        className={`flex min-h-7 w-full min-w-0 items-center gap-1.5 rounded text-left ${collapsible ? "hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]" : "cursor-default"}`}
      >
        <Brain size={13} className="shrink-0 text-[var(--quiet)]" />
        {/* Not `font-medium` in `--text`: a row label is chrome, and the thought
            itself below it is the content. Weighting the label at full contrast
            made a long reasoning trace read as a wall of headings. */}
        <span className="shrink-0 text-[var(--muted)]">Thought for {formatSeconds(seconds)}</span>
        {step.running && live && (
          <LoaderCircle size={11} className="shrink-0 animate-spin text-[var(--quiet)]" />
        )}
        {collapsible && caret}
      </button>
      {collapsible && open && (
        // Aligned to the label, not to where the caret used to sit on the left.
        // The trace belongs to the thought, and it reads as belonging to the row
        // above it when it starts where that row's text starts.
        <div className="markdown-content pb-1 pl-[26px] pt-0.5">{renderMarkdown(step.text)}</div>
      )}
    </li>
  );
}

/**
 * One tool call, or a run of them folded together.
 *
 * A run carries its count rather than the individual paths, and the paths stay
 * one disclosure away — collapsing saves the reader three near-identical lines
 * without hiding what was read.
 *
 * **Every row opens, including a run of one.** The disclosure is offered when
 * there is something behind it, which is the call's own output: the text the
 * model was handed. Previously only a folded run was clickable, so a single call
 * — the common case, and the one where a command's output or a read's contents
 * are most worth seeing — was the one shape with no way to look at what came
 * back. The output is rendered in a monospace block rather than as markdown,
 * because it is the tool's literal text: a diff, a stack trace or a directory
 * listing is not prose, and formatting it would misrepresent what the model saw.
 */
function ToolRow({ steps, live }: { steps: ToolStep[]; live: boolean }) {
  const [expanded, setExpanded] = useState(false);
  const first = steps[0];
  const running = steps.some((step) => step.running);
  const seconds = useRunSeconds(steps, live);
  const failed = steps.some((step) => step.failed);
  const outcome = groupOutcome(steps);
  // What is behind the disclosure: each call's diff where it has one, and its
  // own output otherwise. A write's diff replaces its output rather than sitting
  // beside it, because the output for an edit is the lines that landed and the
  // diff is the same lines plus the ones they replaced -- showing both repeats
  // every added line twice.
  const outputs = steps.flatMap((step) => {
    if (step.diff) return [];
    return step.output ? [{ step, output: step.output }] : [];
  });
  const diffs = steps.flatMap((step) => (step.diff ? [{ step, diff: step.diff }] : []));
  // A search has nothing to open until its results land, so the row is only a
  // disclosure once they have: a caret over a running search would open nothing.
  const hasResults = steps.some((step) => step.search);
  const collapsible = steps.length > 1 || outputs.length > 0 || diffs.length > 0 || hasResults;

  const label = (
    <>
      <ToolIcon tool={first.tool} />
      <span className="shrink-0 text-[var(--muted)]">{collapsible ? groupLabel(steps) : first.label}</span>
      {/* Content-width, and truncating rather than growing. `flex-1` here was
          wrong: it stretched the detail to fill the row, so the figures and the
          caret were pushed to the far edge and a short command left a hand's
          width of blank space between the command and its own outcome. `min-w-0`
          with no growth lets it take what it needs and still shrink on a long
          path, which keeps the figures beside the text rather than off-row. */}
      <span className="min-w-0 truncate text-[var(--quiet)]">
        {collapsible ? groupDetail(steps) : first.detail}
      </span>
    </>
  );
  // Everything after the text lives inside the button, so the whole row is one
  // target. Split across two elements left the caret in the middle with figures
  // on either side of it, which read as belonging to two different things.
  const figures = (
    <>
      {outcome.text && (
        <span className={`shrink-0 text-[var(--quiet)] ${outcome.failed ? "text-[var(--danger)]" : ""}`}>
          {outcome.text}
        </span>
      )}
      {seconds > 0 && <span className="shrink-0 text-[var(--quiet)]">{formatSeconds(seconds)}</span>}
      {running && live && (
        <LoaderCircle size={11} className="shrink-0 animate-spin text-[var(--accent)]" />
      )}
    </>
  );
  // Last on the row, as on the thought row. The caret is the terminator of the
  // line rather than something sitting in the middle of it: everything the reader
  // wants to know is in front of it, and it says "there is more behind this".
  const caret = (
    <ChevronRight
      size={12}
      aria-hidden
      className={`shrink-0 transition-transform ${expanded ? "rotate-90" : ""}`}
    />
  );

  return (
    <li className="text-[12px] leading-6 text-[var(--muted)]">
      <div className="flex items-center gap-1.5 py-0.5">
        {collapsible ? (
          <button
            type="button"
            onClick={() => setExpanded((value) => !value)}
            aria-expanded={expanded}
            className="flex min-h-7 min-w-0 flex-1 items-center gap-1.5 rounded text-left hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
          >
            {label}
            {figures}
            {caret}
          </button>
        ) : (
          <div className="flex min-h-7 min-w-0 flex-1 items-center gap-1.5">
            {label}
            {figures}
          </div>
        )}
      </div>

      {expanded && (
        // Indented to the tool icon, not to where the caret used to sit on the
        // left. The content belongs to the call it describes.
        <div className="ml-[18px] pb-1 pl-3">
          {steps.length > 1 && (
            <ul className="space-y-0.5 border-l border-[var(--line)] pb-1">
              {steps.map((step, index) => (
                <li key={`${step.tool}-${index}`} className="flex min-h-7 items-center gap-1.5 text-[11px] text-[var(--muted)]">
                  <ToolIcon tool={step.tool} />
                  <span className="shrink-0 text-[var(--muted)]">{step.label}</span>
                  <span className="min-w-0 truncate text-[var(--quiet)]">{step.detail}</span>
                  {step.cached && <span className="shrink-0 text-[var(--quiet)]">cached</span>}
                  {step.failed && <span className="shrink-0 text-[var(--danger)]">{step.outcome}</span>}
                </li>
              ))}
            </ul>
          )}
          {diffs.map(({ step, diff }) => (
            <DiffBlock key={`${step.tool}-diff`} label={step.label} diff={diff} />
          ))}
          {outputs.map(({ step, output }) => (
            <OutputBlock key={`${step.tool}-${output.slice(0, 24)}`} label={step.label} output={output} />
          ))}
          {steps.flatMap((step) => (step.search ? [step.search] : [])).map((search, index) => (
            <SearchResults key={`results-${index}`} search={search} />
          ))}
        </div>
      )}
      {/* Only shown when it failed: the reason is the part worth reading, and a
          failure the user has to expand to discover is a failure they will not
          notice. */}
      {failed && (
        <p className="ml-[18px] py-0.5 pl-3 text-[11px] text-[var(--danger)]">
          {steps.filter((step) => step.failed).map((step) => `${step.label}: ${step.outcome}`).join(" · ")}
        </p>
      )}
    </li>
  );
}

/**
 * The results of one web search.
 *
 * **Inside the panel, behind the row that ran it.** These were a separate message
 * in the transcript, which drew a card above the reply it belonged to — outside
 * the panel that holds every other tool, and in a heavier shape than its
 * neighbours. As row data they open where the reader already is looking.
 *
 * A list rather than prose because that is what it is: title, link, snippet. The
 * snippet is clamped to two lines so one verbose page cannot push the rest of the
 * results out of view.
 */
function SearchResults({
  search,
}: {
  search: NonNullable<ToolStep["search"]>;
}) {
  if (search.results.length === 0) {
    return <p className="py-0.5 text-[11px] text-[var(--quiet)]">No results found.</p>;
  }
  return (
    <ul className="mt-0.5 space-y-1.5">
      {search.results.map((result, index) => (
        <li key={`${result.url}-${index}`}>
          <a
            href={result.url}
            target="_blank"
            rel="noreferrer"
            className="block rounded px-1 py-0.5 hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
          >
            <span className="block truncate text-[11px] font-medium text-[var(--text)]">{result.title}</span>
            <span className="block truncate text-[10px] text-[var(--quiet)]">{result.url}</span>
            {result.snippet && (
              <span className="mt-0.5 block line-clamp-2 text-[11px] leading-5 text-[var(--muted)]">
                {result.snippet}
              </span>
            )}
          </a>
        </li>
      ))}
    </ul>
  );
}

/**
 * What one call returned, in a block the reader can scroll.
 *
 * `whitespace-pre-wrap` rather than `pre` so a long single line wraps instead of
 * forcing the whole panel sideways — a minified file or a one-line error is
 * exactly the case where horizontal scrolling would be worst. The height cap
 * keeps a large output from burying the steps that came after it; the full text
 * is in the transcript either way.
 */
function OutputBlock({ label, output }: { label: string; output: string }) {
  return (
    <figure className="mt-0.5">
      <figcaption className="mb-0.5 text-[10px] uppercase tracking-wide text-[var(--quiet)]">
        {label} output
      </figcaption>
      <pre className="max-h-72 overflow-auto overscroll-contain whitespace-pre-wrap break-words rounded border border-[var(--line)] bg-[var(--rail)] px-2 py-1.5 font-mono text-[11px] leading-5 text-[var(--muted)]">
        {output}
      </pre>
    </figure>
  );
}

/**
 * What a write changed, one line per row.
 *
 * Preferred over the raw output for a write, because the raw output is what the
 * *model* was told and for an edit that is only the lines that landed -- the
 * lines it replaced are exactly what a reader opens the row to find, and they
 * were not in it.
 *
 * Colours come from the existing tokens: `--accent` for an addition and
 * `--danger` for a removal, because those already mean "added" and "wrong"
 * elsewhere in the app. No new token, so the panel restyles with the theme.
 *
 * The sign column is the gutter's whole job: a tinted background alone leaves a
 * removed line and an added one distinguishable only by reading them.
 */
function DiffBlock({ label, diff }: { label: string; diff: ToolDiff }) {
  if (diff.lines.length === 0) return null;
  const tone = (kind: DiffLine["kind"]) =>
    kind === "add"
      ? "bg-[color-mix(in_srgb,var(--accent)_14%,transparent)] text-[var(--text)]"
      : kind === "remove"
        ? "bg-[color-mix(in_srgb,var(--danger)_14%,transparent)] text-[var(--text)]"
        : "text-[var(--muted)]";
  const sign = (kind: DiffLine["kind"]) => (kind === "add" ? "+" : kind === "remove" ? "-" : " ");

  return (
    <figure className="mt-0.5">
      <figcaption className="mb-0.5 text-[10px] uppercase tracking-wide text-[var(--quiet)]">
        {label} changes
      </figcaption>
      <div className="max-h-72 overflow-auto overscroll-contain rounded border border-[var(--line)] bg-[var(--rail)] py-1 font-mono text-[11px] leading-5">
        {diff.lines.map((line, index) => (
          <div key={index} className={`flex ${tone(line.kind)}`}>
            {/* `select-none` so a drag across the block selects the code rather
                than the sign, which is decoration and not part of the text. */}
            <span aria-hidden="true" className="w-4 shrink-0 select-none pl-2 text-[var(--quiet)]">
              {sign(line.kind)}
            </span>
            <pre className="min-w-0 flex-1 whitespace-pre-wrap break-words pr-2">{line.text}</pre>
          </div>
        ))}
        {/* Stated rather than silent: a truncated diff that says nothing reads as
            a file that barely changed, which is the one wrong impression here. */}
        {diff.hidden ? (
          <p className="px-2 pt-1 text-[10px] text-[var(--quiet)]">
            {diff.hidden} more line{diff.hidden === 1 ? "" : "s"} not shown
          </p>
        ) : null}
      </div>
    </figure>
  );
}

/**
 * Folds consecutive finished reads into one row.
 *
 * Only reads fold. Two searches are two questions and merging them would say
 * "2 steps" where what happened was two separate lookups, and a running call
 * stays on its own line so the panel can show it ticking.
 */
function buildRows(steps: ActivityStep[]): Row[] {
  const rows: Row[] = [];
  for (const step of steps) {
    const previous = rows[rows.length - 1];
    const foldable =
      step.kind === "tool" &&
      !step.running &&
      previous?.kind === "tool" &&
      previous.steps.every((entry) => entry.tool === "read_file");
    if (foldable) {
      previous.steps.push(step);
      continue;
    }
    rows.push(step.kind === "thought" ? { kind: "thought", step } : { kind: "tool", steps: [step] });
  }
  return rows;
}

/** The verb phrase for a run of calls. */
function groupLabel(steps: ToolStep[]): string {
  if (steps.length === 1) return steps[0].label;
  return `Read ${steps.length} files`;
}

/** What a run was pointed at: the folder the reads share. */
function groupDetail(steps: ToolStep[]): string {
  if (steps.length === 1) return steps[0].detail;
  const folders = new Set(
    steps.map((step) => {
      const slash = step.detail.lastIndexOf("/");
      return slash === -1 ? "" : step.detail.slice(0, slash);
    }),
  );
  const [folder] = [...folders];
  return folder || steps.map((step) => step.detail).join(", ");
}

/**
 * A failure anywhere outranks the count. "3 files" beside a row where one of
 * them failed is the summary that hides the thing worth knowing.
 */
function groupOutcome(steps: ToolStep[]): { text: string; failed: boolean } {
  if (steps.some((step) => step.failed)) {
    return { text: `${steps.filter((step) => step.failed).length} of ${steps.length} failed`, failed: true };
  }
  if (steps.length === 1) return { text: steps[0].outcome, failed: false };
  return { text: "", failed: false };
}

/**
 * Total time, and the number of tool uses for the header.
 *
 * The time is the caller's measured whole-reply figure whenever there is one,
 * because the step sum is not the reply's duration. Summing steps only counts
 * thinking and tool calls, and a reply that thought briefly and then streamed
 * its answer for ten seconds would report "Worked for 0s" -- which is both
 * wrong and the kind of wrong that makes the whole panel untrustworthy.
 *
 * Two fallbacks, in order of how much they can be trusted:
 *
 * - **Live, no measured figure yet.** Count up from the earliest running step,
 *   so a run in progress still moves. Undercounts the wait before the first
 *   step, which is the honest direction to err in: a panel that counts up from
 *   the first thing that happened cannot overstate it.
 * - **Nothing to sum.** A reply whose every step is closed and which has no
 *   measured total falls back to the step sum rather than reporting nothing,
 *   because a header reading "Worked for" with no figure is worse than a rough
 *   one.
 *
 * The step count covers tool uses only, because that is the figure a reader is
 * scanning for: two thoughts and one read is one step, not three.
 */
function usePanelTotals(
  steps: ActivityStep[],
  live: boolean,
  totalSeconds?: number,
): { seconds: number; toolCount: number } {
  const now = useTick(live, steps);
  let toolCount = 0;
  for (const step of steps) {
    if (step.kind === "tool") toolCount += 1;
  }

  const summed = steps.reduce((total, step) => total + stepSeconds(step, now, live), 0);

  if (totalSeconds !== undefined) return { seconds: totalSeconds, toolCount };
  if (!live) return { seconds: summed, toolCount };

  // Running, so nothing is measured yet. Count from the first step that has
  // begun rather than from zero: a panel showing "0s" for a run that has been
  // going for eight seconds looks broken, whereas starting at the first step
  // only undercounts the untimed moment before it.
  const started = steps
    .map((step) => step.startedAt)
    .filter((value): value is number => value !== undefined);
  const elapsed = started.length > 0 ? Math.max(0, (now - Math.min(...started)) / 1000) : 0;
  return { seconds: Math.max(summed, elapsed), toolCount };
}

/** Seconds for one step, ticking while it runs. */
function stepSeconds(step: ActivityStep, now: number, live: boolean): number {
  if (!step.running) return step.seconds ?? 0;
  if (!live || step.startedAt === undefined) return step.seconds ?? 0;
  // Clamped at zero because the clock is read after the step began, and a
  // negative duration would render as something absurd.
  return Math.max(0, (now - step.startedAt) / 1000);
}

/** Seconds for a row: the whole run, so a group reports what it cost. */
function useRunSeconds(steps: ToolStep[], live: boolean): number {
  const now = useTick(live, steps.some((step) => step.running));
  return steps.reduce((total, step) => total + stepSeconds(step, now, live), 0);
}

/**
 * A clock that advances once a second while something is running.
 *
 * Owned here rather than lifted into the page because it is only ever needed
 * while a row is in flight, and a page-level interval would keep ticking for
 * every conversation on screen for the life of the app. Deliberately
 * self-contained for the same reason `ElapsedStat` is: an earlier version
 * depended on an unrelated interval elsewhere and would have broken silently if
 * that one were slowed.
 */
function useTick(active: boolean, ...deps: unknown[]): number {
  const [now, setNow] = useState(() => Date.now());
  const activeRef = useRef(active);
  activeRef.current = active;
  useEffect(() => {
    // Re-arm only when `active` or the watched values change, so appending a
    // step does not leave a stale interval behind.
    if (!active) return;
    setNow(Date.now());
    const timer = window.setInterval(() => {
      if (!activeRef.current) return;
      setNow(Date.now());
    }, 1000);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, ...deps]);
  return now;
}

/**
 * Seconds the way the header states them: whole seconds, then minutes and
 * seconds. Deliberately not a decimal — "Worked for 15s" is a fact about how
 * long the user waited, and "14.7s" implies a precision nothing here measured.
 */
export function formatSeconds(seconds: number): string {
  const total = Math.max(0, Math.round(seconds));
  if (total < 60) return `${total}s`;
  return `${Math.floor(total / 60)}m ${total % 60}s`;
}
