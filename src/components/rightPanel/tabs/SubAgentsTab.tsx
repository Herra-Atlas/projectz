import { useState, type FormEvent } from "react";
import { ArrowUp, Bot, ChevronRight, LoaderCircle, Square } from "lucide-react";
import ActivityPanel from "../../../pages/chat/ActivityPanel";
import { Markdown } from "../../../pages/chat/Markdown";
import { markdownComponents, MarkdownActionsProvider } from "../../../pages/chat/markdownComponents";
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
 * **There is a composer, and a run can be stopped.** A finished run is a
 * conversation like any other: a further message continues it under the same
 * session, saved and streamed through the ordinary run path. A live row -- its
 * own brief or a follow-up -- can be stopped from the same control the chat page
 * uses, and the stop is scoped to this agent rather than its parent.
 *
 * **The transcript is drawn the way the chat draws one**, down to the activity
 * panel and the markdown, because it *is* the same data. A sub-agent's reply
 * carries the same steps a normal reply does, so reusing the components is not a
 * shortcut here; it is what keeps the two from looking like different features.
 */
export default function SubAgentsTab({ parentSessionId, onOpenUrl }: SubAgentsTabProps) {
  const { runs, activeLive, activeId, transcript, select, close, send, stop, error, loaded } = useSubAgents(true, parentSessionId);
  const [draft, setDraft] = useState("");
  const running = activeLive !== null;

  // The chat's own overrides, so a link and a code block -- including its copy
  // button -- read the same in a run's transcript as in a reply. The provider
  // carries the actions the module-level overrides read (see
  // `markdownComponents`); it renders no element, so it wraps nothing visible.
  const renderMarkdown = (text: string) => (
    <MarkdownActionsProvider openUrl={onOpenUrl}>
      <Markdown text={text} components={markdownComponents} />
    </MarkdownActionsProvider>
  );

  const active = runs.find((run) => run.id === activeId) ?? null;

  /** Sends the draft, restoring it when the run could not be started. */
  const submitDraft = () => {
    const text = draft.trim();
    if (!text || running) return;
    setDraft("");
    void send(text).then((sent) => {
      if (!sent) setDraft(text);
    });
  };
  const submit = (event: FormEvent) => {
    event.preventDefault();
    submitDraft();
  };

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
          {transcript === null && !activeLive ? (
            <div className="flex items-center gap-2 text-sm text-[var(--muted)]">
              <LoaderCircle size={15} className="animate-spin text-[var(--accent)]" />
              Loading the run…
            </div>
          ) : (
            <div className="flex flex-col gap-6">
              {/* A spawned run's brief is its first turn; a follow-up has none of
                  its own, and draws the message that started it instead. */}
              {activeLive?.prompt && <TaskCard text={activeLive.prompt} />}
              {transcript
                ?.filter((message) => message.role === "user")
                .map((message, index) => (
                  <TaskCard key={`user-${index}`} text={message.content} />
                ))}
              {transcript
                ?.filter((message) => message.role === "assistant")
                .map((message, index) => (
                  <div key={`assistant-${index}`}>
                    <ActivityPanel steps={message.activity ?? []} live={false} renderMarkdown={renderMarkdown} totalSeconds={message.metrics?.elapsed_seconds} />
                    <div className="markdown-content text-[14px] leading-7 text-[var(--text)]">{renderMarkdown(message.content)}</div>
                    <FileChanges steps={message.activity ?? []} live={false} />
                  </div>
                ))}
              {activeLive && (
                <>
                  {activeLive.userText && <UserTurn text={activeLive.userText} />}
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
                </>
              )}
            </div>
          )}
        </div>

        {/* The chat composer's card, for this conversation. The field is closed
            while the agent works -- there is one run to watch and one thing worth
            pressing -- and the button becomes Stop, the same control the chat page
            shows for its own run. */}
        <form onSubmit={submit} className="shrink-0 border-t border-[var(--line)] px-3 py-2.5">
          <div className="flex items-end gap-1 rounded-lg border border-[var(--line)] bg-[var(--panel)] px-2 py-1.5 shadow-[0_8px_24px_-14px_rgba(0,0,0,0.5)] transition-colors focus-within:border-[color-mix(in_srgb,var(--accent)_30%,var(--line))]">
            <textarea
              aria-label="Message this agent"
              rows={1}
              value={draft}
              onChange={(event) => setDraft(event.target.value)}
              onKeyDown={(event) => {
                if (event.key !== "Enter" || event.shiftKey) return;
                event.preventDefault();
                submitDraft();
              }}
              disabled={running}
              placeholder={running ? "The agent is working…" : "Message this agent…"}
              className="max-h-[9.5rem] min-h-9 min-w-0 flex-1 resize-none overflow-y-auto bg-transparent px-1.5 py-2 text-[14px] leading-6 text-[var(--text)] outline-none placeholder:text-[var(--quiet)] disabled:cursor-not-allowed disabled:opacity-60"
            />
            {running ? (
              <button
                type="button"
                onClick={() => void stop()}
                aria-label="Stop agent"
                title="Stop agent"
                className="grid size-9 shrink-0 place-items-center rounded-full bg-[var(--raised)] text-[var(--text)] transition-colors hover:bg-[color-mix(in_srgb,var(--accent)_20%,var(--raised))] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
              >
                <Square size={14} fill="currentColor" />
              </button>
            ) : (
              <button
                type="submit"
                disabled={!draft.trim()}
                aria-label="Send message"
                title="Send message"
                className="grid size-9 shrink-0 place-items-center rounded-full bg-[var(--text)] text-[var(--page)] transition-colors hover:bg-[var(--accent)] hover:text-[var(--accent-ink)] disabled:cursor-not-allowed disabled:opacity-35 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
              >
                <ArrowUp size={18} strokeWidth={2.5} />
              </button>
            )}
          </div>
        </form>

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

/** The user's turn in a follow-up, drawn as theirs rather than as the agent's brief. */
function UserTurn({ text }: { text: string }) {
  return (
    <article className="ml-auto max-w-[85%] rounded-lg bg-[var(--raised)] px-3 py-2">
      <p className="whitespace-pre-wrap break-words text-[13px] leading-6 text-[var(--text)]">{text}</p>
    </article>
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
