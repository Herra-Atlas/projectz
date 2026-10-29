import { useEffect, useMemo, useState } from "react";
import { Brain, ChevronRight, LoaderCircle } from "lucide-react";
import ToolIcon from "../../components/ToolIcon";
import { formatTokens } from "../../features/chat/sessionStats";
import type { ActivityStep, DiffLine, NoteStep, ThoughtStep, ToolDiff, ToolStep, ToolStepStats } from "../../features/chat/types";

/**
 * One row of the panel: one tool call, one thought, or one note.
 *
 * A tool call is a step, so it is its own row. Nothing is folded: the header
 * counts calls, and folding two calls behind one "Read 2 files" line made the
 * header and the list disagree about how many steps a reply took.
 */
type Row =
  | { kind: "tool"; step: ToolStep }
  | { kind: "thought"; step: ThoughtStep }
  | { kind: "note"; step: NoteStep };

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
  /**
   * When the run being shown began, as a `Date.now()` reading.
   *
   * The live clock counts from here rather than from the earliest step: every
   * close path deletes a step's `startedAt`, so after the first step finished the
   * only start left was the one currently open, and the header read the *closed*
   * step sum instead -- which advances only when another step closes. That is why
   * "Working for" sat still while the model thought and jumped when a tool ran.
   */
  startedAt?: number;
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
export default function ActivityPanel({ steps, live, renderMarkdown, totalSeconds, startedAt }: ActivityPanelProps) {
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

  const settled = useMemo(() => closeSuperseded(mergeRepeatedThoughts(steps)), [steps]);
  const rows = useMemo(() => buildRows(settled), [settled]);
  const totals = usePanelTotals(settled, live, totalSeconds, startedAt);

  if (steps.length === 0) return null;

  return (
    <section className="mb-3 min-w-0 max-w-2xl overflow-hidden" aria-label="Reasoning and tools">
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
        {/* Present tense while the run is in flight: "Working for" is the truth
            as the clock counts, and "Worked for" only once it has stopped. */}
        <span className="text-[var(--muted)]">
          {live ? "Working for" : "Worked for"} {formatSeconds(totals.seconds)}
        </span>
        {/* Tool uses only: one call is one step, and one step is one row, so the
            figure always matches the list beneath it. A thought is listed but not
            counted -- counting it made a reply with a single read read as "2
            steps". Hidden entirely when there were no tools, so a reply that only
            reasoned does not claim it ran something. */}
        {totals.toolCount > 0 && (
          <span className="text-[var(--quiet)]">
            · {totals.toolCount === 1 ? "1 step" : `${totals.toolCount} steps`}
          </span>
        )}
      </button>

      {open && (
        <ol className="mt-1 space-y-0.5 border-l border-[var(--line)] pl-3">
          {rows.map((row, index) =>
            row.kind === "thought" ? (
              <ThoughtRow key={`thought-${index}`} step={row.step} live={live} renderMarkdown={renderMarkdown} />
            ) : row.kind === "note" ? (
              <NoteRow key={`note-${index}`} step={row.step} renderMarkdown={renderMarkdown} />
            ) : (
              <ToolRow key={`tool-${index}`} step={row.step} live={live} />
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
  const now = useTick(live && step.running);
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
 * Prose the model wrote in the middle of the work.
 *
 * Rendered as markdown, like the answer and a thought, because it is the model's
 * own writing and it is full of the same code fences and lists. Sized and coloured
 * like the answer rather than like a row label, because it *is* text to read -- it
 * is only here rather than in the answer's bubble because the reply went on
 * working afterwards, and the stored answer keeps the final round alone.
 *
 * No duration, no caret, no spinner: a note is complete the moment it is moved
 * here, so there is nothing behind it to open and no clock to show.
 */
function NoteRow({
  step,
  renderMarkdown,
}: {
  step: NoteStep;
  renderMarkdown: (text: string) => React.ReactNode;
}) {
  return (
    <li className="markdown-content py-0.5 text-[13px] leading-6 text-[var(--muted)]">
      {renderMarkdown(step.text)}
    </li>
  );
}

/**
 * One tool call.
 *
 * One call is one row and one step, so the header's count and the list always
 * agree. Reads are no longer folded into a "Read 2 files" line: that line was one
 * row but counted as two steps above it.
 *
 * **Every row opens.** The disclosure is offered when there is something behind
 * it, which is the call's own output: the text the model was handed. Previously
 * only a folded run was clickable, so a single call — the common case, and the one
 * where a command's output or a read's contents are most worth seeing — was the
 * one shape with no way to look at what came back. The output is rendered in a
 * monospace block rather than as markdown, because it is the tool's literal text:
 * a diff, a stack trace or a directory listing is not prose, and formatting it
 * would misrepresent what the model saw.
 *
 * **A failure reads red on the row, not under it.** The reason used to be printed
 * below every failed row, so a run with a refusal or two carried a wall of error
 * text the reader scrolled past. Now the icon and the label take `--danger` and
 * the reason itself sits behind the same disclosure as a success's output: the
 * signal is on the row, the detail is one click away.
 */
function ToolRow({ step, live }: { step: ToolStep; live: boolean }) {
  const [expanded, setExpanded] = useState(false);
  const running = step.running;
  // Only a running row needs a clock. A finished one reports the backend's
  // measured figure, so ticking for it would re-render the row once a second for
  // nothing.
  const now = useTick(live && running);
  const seconds = stepSeconds(step, now, live);
  // A failure's reason lives behind the disclosure rather than on the row, so
  // the collapsed line only has to say red.
  const outcome = step.failed ? "" : step.outcome;
  // A write's diff replaces its output rather than sitting beside it, because the
  // output for an edit is the lines that landed and the diff is the same lines
  // plus the ones they replaced -- showing both repeats every added line twice.
  const hasOutput = step.diff === undefined && Boolean(step.output);
  // A search has nothing to open until its results land, so the row is only a
  // disclosure once they have: a caret over a running search would open nothing.
  // A failure opens too, because its reason lives behind the row.
  const collapsible = hasOutput || step.diff !== undefined || step.search !== undefined || step.failed;

  const label = (
    <>
      <ToolIcon tool={step.tool} className={step.failed ? "text-[var(--danger)]" : undefined} />
      <span className={`shrink-0 ${step.failed ? "text-[var(--danger)]" : "text-[var(--muted)]"}`}>
        {step.label}
      </span>
      {/* Content-width, and truncating rather than growing. `flex-1` here was
          wrong: it stretched the detail to fill the row, so the figures and the
          caret were pushed to the far edge and a short command left a hand's
          width of blank space between the command and its own outcome. `min-w-0`
          with no growth lets it take what it needs and still shrink on a long
          path, which keeps the figures beside the text rather than off-row. */}
      <span className="min-w-0 truncate text-[var(--quiet)]">{step.detail}</span>
    </>
  );
  // Everything after the text lives inside the button, so the whole row is one
  // target. Split across two elements left the caret in the middle with figures
  // on either side of it, which read as belonging to two different things.
  const figures = (
    <>
      {outcome && (
        // Shrinks and truncates like the detail above it. It used to be
        // `shrink-0`, so its width was its content's -- a long command's first
        // line or a file's created path pushed the row past the bubble and the
        // transcript grew a horizontal scrollbar.
        <span className="min-w-0 truncate text-[var(--quiet)]">{outcome}</span>
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
      <div className="flex min-w-0 items-center gap-1.5 py-0.5">
        {collapsible ? (
          <button
            type="button"
            onClick={() => setExpanded((value) => !value)}
            aria-expanded={expanded}
            // The row clips its detail to one line, so a long command is
            // unreadable at a glance. The title carries the whole of it, and
            // TooltipLayer draws it without spending the width the rest of the
            // row needs.
            title={step.detail}
            className="flex min-h-7 min-w-0 flex-1 items-center gap-1.5 rounded text-left hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
          >
            {label}
            {figures}
            {caret}
          </button>
        ) : (
          <div className="flex min-h-7 min-w-0 flex-1 items-center gap-1.5" title={step.detail}>
            {label}
            {figures}
          </div>
        )}
      </div>

      {expanded && (
        // Indented to the tool icon, not to where the caret used to sit on the
        // left. The content belongs to the call it describes.
        <div className="ml-[18px] pb-1 pl-3">
          {step.failed && (
            // A failed call has no output stored, so its reason is the one thing
            // behind the disclosure. The red label and icon are the signal while
            // the row is collapsed.
            <p className="py-0.5 text-[11px] text-[var(--danger)]">{step.outcome}</p>
          )}
          {step.diff ? (
            <DiffBlock label={step.label} diff={step.diff} />
          ) : step.output ? (
            <OutputBlock label={step.label} output={step.output} stats={step.stats} />
          ) : null}
          {step.search && <SearchResults search={step.search} />}
        </div>
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
function OutputBlock({ label, output, stats }: { label: string; output: string; stats?: ToolStepStats }) {
  return (
    <figure className="mt-0.5">
      <figcaption className="mb-0.5 text-[10px] uppercase tracking-wide text-[var(--quiet)]">
        {label} output
      </figcaption>
      <pre className="max-h-72 overflow-auto overscroll-contain whitespace-pre-wrap break-words rounded border border-[var(--line)] bg-[var(--rail)] px-2 py-1.5 font-mono text-[11px] leading-5 text-[var(--muted)]">
        {output}
      </pre>
      {/* Only a delegated run carries these, and they belong to the output the way
          a table's totals belong under it -- so they sit inside the block rather
          than on the row, where they would compete with the outcome beside it. */}
      {stats && <SubagentStats stats={stats} />}
    </figure>
  );
}

/**
 * What a delegated run cost and did, under its report.
 *
 * The same facts the session overview states for a conversation, stated for one
 * sub-agent run so it can be judged from the reply that spawned it. Deliberately a
 * plain line rather than a grid: it is one row among a run's steps, not a page of
 * its own, and a box here would outrank the report it describes.
 *
 * Every figure is the run's own -- spend across all its rounds, not its last
 * request. `tokens` is dropped when the provider reported no usage rather than
 * shown as zero, because an unknown cost must not read as a free run.
 */
function SubagentStats({ stats }: { stats: ToolStepStats }) {
  const parts = [
    stats.tokens !== undefined ? `${formatTokens(stats.tokens)} tokens` : null,
    `${stats.toolUses} tool ${stats.toolUses === 1 ? "use" : "uses"}`,
    `${stats.turns} ${stats.turns === 1 ? "turn" : "turns"}`,
    formatSeconds(stats.seconds),
  ].filter((part): part is string => part !== null);
  return (
    <p className="mt-1 border-t border-[var(--line)] pt-1 text-[11px] text-[var(--quiet)] tabular-nums">
      {parts.join(" · ")}
    </p>
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
 * Drops a thought that is the same thought as the one before it.
 *
 * One thought used to be written down twice: the streamed row closed as the
 * answer began, and the backend's whole-text segment then appended beside it
 * because the row above was no longer `running`. The fold is fixed, but a
 * transcript saved before that carries both copies, so the panel has to be
 * able to read one -- and equal text in two adjacent thoughts is never two
 * thoughts: a second stretch of thinking always follows a tool call or an
 * answer, so it is never the next row.
 *
 * The row kept is the one with a duration. The duplicate is the copy that lost
 * it, so keeping the first and its measured figure is what makes an old reply
 * read like a new one.
 */
function mergeRepeatedThoughts(steps: ActivityStep[]): ActivityStep[] {
  const merged: ActivityStep[] = [];
  for (const step of steps) {
    const previous = merged[merged.length - 1];
    if (step.kind === "thought" && previous?.kind === "thought" && previous.text === step.text) {
      if ((previous.seconds ?? 0) < (step.seconds ?? 0)) merged[merged.length - 1] = step;
      continue;
    }
    merged.push(step);
  }
  return merged.length === steps.length ? steps : merged;
}

/**
 * Ends steps that a later step has already superseded.
 *
 * Nothing in this panel runs two rows at once: a step is appended after the one
 * before it has finished, which is why only the last step can still be running.
 * A `running` flag left on an earlier step is therefore a missed close, not
 * concurrency -- and it shows: the thought keeps counting while the tool it
 * handed over to is working, so a row claims to be thinking long after it
 * stopped.
 *
 * Closing it at the moment the next step began keeps the figure honest and
 * needs no event the backend does not already send.
 */
function closeSuperseded(steps: ActivityStep[]): ActivityStep[] {
  let changed = false;
  const settled = steps.map((step, index) => {
    if (step.kind === "note" || !step.running || step.startedAt === undefined) return step;
    const follower = steps
      .slice(index + 1)
      .find(
        (entry): entry is ToolStep | ThoughtStep =>
          entry.kind !== "note" && entry.startedAt !== undefined,
      );
    if (follower?.startedAt === undefined) return step;
    changed = true;
    return {
      ...step,
      running: false,
      seconds: Math.max(0, (follower.startedAt - step.startedAt) / 1000),
    };
  });
  return changed ? settled : steps;
}

/**
 * One row per step. A tool call is a step, so it gets its own line.
 *
 * Reads used to fold into a "Read 2 files" row, which counted as one line but
 * two calls in the header. The two figures disagreed for the same run -- the list
 * showed four rows and the header said eight steps. One call, one row, one step
 * removes the disagreement by construction.
 */
function buildRows(steps: ActivityStep[]): Row[] {
  return steps.map((step) => {
    if (step.kind === "tool") return { kind: "tool", step };
    if (step.kind === "thought") return { kind: "thought", step };
    return { kind: "note", step };
  });
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
 * - **Live, no measured figure yet.** The wall clock from the moment the run
 *   started, so the header counts up every second whether the model is thinking,
 *   streaming or reading. Counting from the earliest *live* step instead was the
 *   bug: closing a step deletes its start, so the only start left was the one
 *   currently open, and `Math.max(summed, elapsed)` then printed the closed step
 *   sum -- a number that only moves when a step closes.
 * - **Nothing to sum.** A reply whose every step is closed and which has no
 *   measured total falls back to the step sum rather than reporting nothing,
 *   because a header reading "Worked for" with no figure is worse than a rough
 *   one.
 *
 * **Running steps are deliberately not summed.** They used to be, each counting
 * from its own start, and because those windows overlap the total was their sum
 * rather than their span -- a long run with a dozen rows left open reported
 * "Worked for 85m" for work that took minutes. A step that is still running
 * contributes nothing to the sum; the wall clock is what covers it, and it is a
 * span by construction.
 *
 * The step count is the number of tool calls -- one call, one step -- which is
 * also exactly the number of tool rows the list draws, so the header and the list
 * cannot disagree. A thought is listed but never counted: two thoughts and one
 * read is one step, not three.
 */
function usePanelTotals(
  steps: ActivityStep[],
  live: boolean,
  totalSeconds?: number,
  startedAt?: number,
): { seconds: number; toolCount: number } {
  const now = useTick(live);
  let toolCount = 0;
  let summed = 0;
  for (const step of steps) {
    if (step.kind === "tool" && !step.synthetic) toolCount += 1;
    // A step that is still running adds nothing here: the wall clock below covers
    // its time, and adding it again is what double-counted it.
    if (step.kind !== "note" && !step.running) summed += step.seconds ?? 0;
  }

  if (totalSeconds !== undefined) return { seconds: totalSeconds, toolCount };
  if (!live) return { seconds: summed, toolCount };

  // The run's own start, when the caller has one. This is the whole figure while
  // live: it includes the wait before the first step and does not reset when a
  // step closes.
  if (startedAt !== undefined && startedAt > 0) {
    return { seconds: Math.max(0, (now - startedAt) / 1000), toolCount };
  }

  // No anchor (a run adopted under an older shape): start from the first step that
  // has begun rather than from zero, so the header is not stuck at "0s".
  const started = steps
    .filter((step) => step.kind !== "note")
    .map((step) => step.startedAt)
    .filter((value): value is number => value !== undefined);
  const elapsed = started.length > 0 ? Math.max(0, (now - Math.min(...started)) / 1000) : 0;
  return { seconds: Math.max(summed, elapsed), toolCount };
}

/** Seconds for one step, ticking while it runs. */
function stepSeconds(step: ActivityStep, now: number, live: boolean): number {
  // A note has no duration: it is prose, not work that took time.
  if (step.kind === "note") return 0;
  if (!step.running) return step.seconds ?? 0;
  if (!live || step.startedAt === undefined) return step.seconds ?? 0;
  // Clamped at zero because the clock is read after the step began, and a
  // negative duration would render as something absurd.
  return Math.max(0, (now - step.startedAt) / 1000);
}

/**
 * A clock that advances once a second while something is running.
 *
 * Once a second, matching the label: `formatSeconds` states whole seconds, so a
 * faster tick would only re-render the row to print the same figure. The
 * interval runs only while a row is in flight, so it costs nothing once the
 * reply has landed.
 *
 * Owned here rather than lifted into the page because it is only ever needed
 * while a row is in flight, and a page-level interval would keep ticking for
 * every conversation on screen for the life of the app. Deliberately
 * self-contained for the same reason `ElapsedStat` is: an earlier version
 * depended on an unrelated interval elsewhere and would have broken silently if
 * that one were slowed.
 */
function useTick(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    // Read the clock on the way in as well as on every tick: a row that starts
    // between two ticks should not report the time of the previous one.
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [active]);
  return now;
}

/**
 * A duration the way the panel states it: whole seconds, then minutes and
 * seconds.
 *
 * Whole seconds throughout, so every figure in the panel counts in the same
 * unit. Milliseconds used to be shown below two seconds to tell an instant tool
 * from a slow one, but a number that jumps by a thousand at a time does not read
 * as counting -- it reads as broken. Seconds everywhere does not have that
 * problem, at the cost of an instant call and a slow one both reading "0s".
 */
export function formatSeconds(seconds: number): string {
  const total = Math.round(Math.max(0, seconds));
  if (total < 60) return `${total}s`;
  return `${Math.floor(total / 60)}m ${total % 60}s`;
}
