import { useMemo } from "react";
import { Bot, ChevronRight, LoaderCircle } from "lucide-react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import ActivityPanel from "../../../pages/chat/ActivityPanel";
import FileChanges from "../../../pages/chat/FileChanges";
import { MARKDOWN_STYLES } from "../../../pages/chat/markdownStyles";
import LiveSpinner from "../../LiveSpinner";
import { useSubAgents } from "../../../features/subagents/useSubAgents";

type SubAgentsTabProps = {
  /**
   * The conversation that is open, so the list follows it.
   *
   * `null` shows every run, which is what "no conversation open" should mean
   * rather than an empty panel. Runs started from a chat are grouped under it by
   * the backend's filter, so switching conversation switches the list.
   */
  parentSessionId: string | null;
  /** Opens a link from a run's answer in the panel's browser. */
  onOpenUrl: (url: string) => void;
};

/**
 * The runs delegated from here, and the workings of whichever is open.
 *
 * **Two states, like the Files view.** The list of runs is the default; opening
 * one replaces it with that run's transcript, and a back control in the header
 * leads to the list again.
 *
 * **A running run is drawn as it happens.** Its events are folded into activity
 * and text while it streams, so the panel shows the agent thinking, calling tools
 * and answering rather than nothing until it is done. The mark on its row -- the
 * same spinner the sidebar uses for a writing session -- replaces the agent icon,
 * so a run in flight is recognisable at a glance.
 *
 * **Read-only, and deliberately so.** A sub-agent has no composer: it was given
 * one task and finished it, and talking back to it after the fact would either do
 * nothing or start work the parent never asked for. The panel is a record — what
 * the agent read, ran and answered — not a second chat.
 *
 * **The transcript is drawn the way the chat draws one**, down to the activity
 * panel and the markdown, because it *is* the same data. A sub-agent's reply
 * carries the same steps a normal reply does, so reusing the components is not a
 * shortcut here; it is what keeps the two from looking like different features.
 */
export default function SubAgentsTab({ parentSessionId, onOpenUrl }: SubAgentsTabProps) {
  const { runs, activeLive, activeId, transcript, select, close, error, loaded } = useSubAgents(true, parentSessionId);

  const markdownComponents = useMemo(
    () => ({
      a: ({ href, children, ...props }: React.ComponentProps<"a">) => (
        <a
          {...props}
          href={href}
          onClick={(event) => {
            event.preventDefault();
            if (href) onOpenUrl(href);
          }}
        >
          {children}
        </a>
      ),
    }),
    [onOpenUrl],
  );
  const renderMarkdown = (text: string) => (
    <ReactMarkdown remarkPlugins={[remarkGfm]} components={markdownComponents}>{text}</ReactMarkdown>
  );

  const active = runs.find((run) => run.id === activeId) ?? null;

  // The one open run: a header that leads back to the list, above its transcript.
  if (active) {
    return (
      <div className="flex min-h-0 flex-1 flex-col">
        <style>{MARKDOWN_STYLES}</style>

        {/* A back control rather than a tab, the same choice the Files view makes:
            the list and one run are two states of one view, and a tab strip would
            imply they could both be open at once, which they cannot. The title is
            the run's label, so the reader knows which agent they are in. */}
        <div className="flex min-h-9 shrink-0 items-center gap-1 border-b border-[var(--line)] px-2">
          <button
            type="button"
            onClick={close}
            aria-label="Back to sub agents"
            title="Back to sub agents"
            className="grid size-6 shrink-0 place-items-center rounded text-[var(--muted)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
          >
            <ChevronRight size={14} className="rotate-180" />
          </button>
          <RunMark id={active.id} running={active.running} size={13} />
          <span className="min-w-0 flex-1 truncate text-xs text-[var(--muted)]">{active.title}</span>
          <span className="shrink-0 text-[11px] text-[var(--quiet)]">{formatWhen(active.updatedAt)}</span>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">
          {activeLive ? (
            <div className="flex flex-col gap-6">
              <TaskCard text={activeLive.prompt} />
              <div>
                {/* The work behind the answer, ticking live while it happens. */}
                <ActivityPanel steps={activeLive.activity} live renderMarkdown={renderMarkdown} />
                {activeLive.text ? (
                  <div className="markdown-content text-[14px] leading-7 text-[var(--text)]">{renderMarkdown(activeLive.text)}</div>
                ) : (
                  // Nothing to read yet -- the agent is still thinking or calling
                  // tools. Said plainly rather than left blank, so an open run does
                  // not read as a run with no answer.
                  <p className="flex items-center gap-2 text-[13px] text-[var(--muted)]">
                    <LiveSpinner gradientId={`pz-spin-${activeLive.id}-body`} size={14} />
                    Working…
                  </p>
                )}
                <FileChanges steps={activeLive.activity} live />
              </div>
            </div>
          ) : transcript === null ? (
            <div className="flex items-center gap-2 text-sm text-[var(--muted)]">
              <LoaderCircle size={15} className="animate-spin text-[var(--accent)]" />
              Loading the run…
            </div>
          ) : (
            <div className="flex flex-col gap-6">
              {transcript
                .filter((message) => message.role === "user")
                .map((message, index) => (
                  <TaskCard key={`user-${index}`} text={message.content} />
                ))}
              {transcript
                .filter((message) => message.role === "assistant")
                .map((message, index) => (
                  <div key={`assistant-${index}`}>
                    <ActivityPanel steps={message.activity ?? []} live={false} renderMarkdown={renderMarkdown} totalSeconds={message.metrics?.elapsed_seconds} />
                    <div className="markdown-content text-[14px] leading-7 text-[var(--text)]">{renderMarkdown(message.content)}</div>
                    <FileChanges steps={message.activity ?? []} live={false} />
                  </div>
                ))}
            </div>
          )}
        </div>

        {error && <p role="alert" className="border-t border-[var(--line)] px-5 py-2 text-[11px] text-[var(--danger)]">{error}</p>}
      </div>
    );
  }

  // The default state: every run delegated from this chat, as a plain list.
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {runs.length === 0 ? (
        <div className="flex h-full flex-col justify-center px-5 py-8">
          <h3 className="text-sm font-medium text-[var(--text)]">No sub agents yet</h3>
          <p className="mt-1 text-[13px] text-[var(--muted)]">
            {loaded
              ? "When an agent in this chat delegates a task to another agent, its run appears here — the whole transcript, exactly as it happened."
              : "Loading…"}
          </p>
        </div>
      ) : (
        /* A run is named by its label and the time it finished, which is what
           tells two runs of the same task apart. A running one wears the spinner
           instead of the agent icon, so it is unmistakable; the trailing chevron
           is the promise the header keeps. */
        <ul className="min-h-0 flex-1 overflow-y-auto">
          {runs.map((run) => (
            <li key={run.id}>
              <button
                type="button"
                onClick={() => void select(run.id)}
                className="flex w-full items-center gap-2 border-b border-[var(--line)] px-5 py-2.5 text-left text-[13px] transition-colors last:border-b-0 hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-[var(--accent)]"
              >
                <RunMark id={run.id} running={run.running} />
                <span className="min-w-0 flex-1 truncate text-[var(--text)]">{run.title}</span>
                <span className="shrink-0 text-[11px] text-[var(--quiet)]">{formatWhen(run.updatedAt)}</span>
                <ChevronRight size={14} className="shrink-0 text-[var(--quiet)]" />
              </button>
            </li>
          ))}
        </ul>
      )}

      {error && <p role="alert" className="border-t border-[var(--line)] px-5 py-2 text-[11px] text-[var(--danger)]">{error}</p>}
    </div>
  );
}

/**
 * A run's mark: the writing spinner while it is live, the agent icon otherwise.
 *
 * The spinner is the sidebar's own mark for a session that is writing, reused so
 * a run in flight looks the same wherever it is being watched. Reduced motion is
 * respected by the shared `animate-spin` class, and the label names the state for
 * a screen reader, which the icon alone does not.
 */
function RunMark({ id, running, size = 14 }: { id: string; running: boolean; size?: number }) {
  if (!running) return <Bot size={size} className="shrink-0 text-[var(--quiet)]" />;
  return (
    <span role="status" aria-label="Agent is working" title="Agent is working" className="grid size-4 shrink-0 place-items-center text-[var(--accent)]">
      <LiveSpinner gradientId={`pz-spin-${id}`} size={size} />
    </span>
  );
}

/** The brief a run was given, shown above its answer. */
function TaskCard({ text }: { text: string }) {
  return (
    <article className="rounded-lg border border-[var(--line)] bg-[var(--rail)] px-3 py-2">
      <p className="mb-1 text-[10px] font-semibold uppercase tracking-wide text-[var(--quiet)]">Task</p>
      <p className="whitespace-pre-wrap break-words text-[13px] leading-6 text-[var(--muted)]">{text}</p>
    </article>
  );
}

/**
 * When a run finished, short enough for a list row.
 *
 * Local time rather than the stored UTC string: the reader wants to know when it
 * happened to them, and the timestamp is stored in UTC for ordering, not for
 * reading.
 */
function formatWhen(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  const sameDay = new Date().toDateString() === date.toDateString();
  return sameDay
    ? date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
    : date.toLocaleDateString([], { month: "short", day: "numeric" });
}
