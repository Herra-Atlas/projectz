/**
 * What a reply did before it answered.
 *
 * Stored on the assistant message rather than held in component state, because
 * the transcript is the record. A panel that only existed while the reply was
 * streaming would be gone by the time the user scrolled back to ask what the
 * agent had been doing.
 */
export type ActivityStep = ToolStep | ThoughtStep | NoteStep;

/**
 * One tool call.
 *
 * A single call spans two events: it appears when the model asks for it and is
 * completed when it finishes. `running` is what separates a row the user can
 * watch from one that is done, and it is why the two events exist rather than
 * the row being built once the tool has already returned.
 */
export type ToolStep = {
  kind: "tool";
  /** The tool as the model named it, e.g. `read_file`. Stable and unique. */
  tool: string;
  /** What it was pointed at: a path, a query, a command line. */
  detail: string;
  /** "Read file", "Find files". Human wording, produced by the backend. */
  label: string;
  /** What came of it, or why it failed. */
  outcome: string;
  failed: boolean;
  /** Seconds the call took. Absent while it is still running, and for a call
      that never ran — an absent duration must not read as a genuinely instant
      result, which is a different claim. */
  seconds?: number;
  running: boolean;
  /**
   * True when the result came from the tool cache, or when the call never ran
   * because its arguments were malformed. Both are reported rather than hidden:
   * a repeat read costs nothing the second time and the panel should say so.
   */
  cached?: boolean;
  /**
   * What the tool returned, as the model was given it.
   *
   * Capped by the backend at a much lower limit than the model sees, because
   * this copy is stored on the message and re-loaded with the transcript. Absent
   * rather than empty when there is nothing worth opening: a one-line result is
   * already on the row, and a failure carries its reason there instead.
   *
   * Present so a user can read what the agent actually saw rather than a summary
   * of it — the distinction matters when the two disagree, which is exactly when
   * someone is auditing the run.
   */
  output?: string;
  /**
   * The results of a web search, attached to the `search_web` call that ran it.
   *
   * On the step rather than as a separate message, so a search is drawn where
   * every other tool is — inside the panel, behind a disclosure — instead of as a
   * card floating above the reply it belongs to.
   */
  search?: { query: string; results: { title: string; url: string; snippet: string }[] };
  /**
   * Local timestamp when the call began, used to tick a running row.
   *
   * Frontend-local rather than backend-supplied on purpose: the clock has to
   * advance between events, and a figure sent once with the first event could
   * not do that.
   */
  startedAt?: number;
  /**
   * What this call changed, for a write.
   *
   * Absent on everything else, including a read: a diff only exists where the
   * tool held both versions of a file while it ran.
   */
  diff?: ToolDiff;
  /**
   * True when this call created the file rather than editing one.
   *
   * Supplied by the backend rather than inferred from `outcome`: a full rewrite
   * of an existing file adds every line and removes none, so the counts alone
   * cannot tell a new file from a replaced one, and reading the outcome's prose
   * for "Created" would break silently the day the wording changes.
   */
  created?: boolean;
  /**
   * The figures behind a delegated run, present only on a `sub_agent` row.
   *
   * A sub-agent's own totals, shown under its report so the run can be judged from
   * the reply that spawned it rather than by opening the Sub agents panel. Set when
   * the run reports back -- see `subagent_result` -- and stored with the step, so a
   * reopened conversation still states them.
   */
  stats?: ToolStepStats;
};

/**
 * What a delegated run cost and did, as its own event reported it.
 *
 * `tokens` counts every round the run made, so it is the run's real spend rather
 * than its final request; it is absent when the provider reported no usage, and an
 * unknown cost is left out rather than shown as zero. The rest are counted from
 * the run's own events.
 */
export type ToolStepStats = {
  /** Tokens billed across the run, absent when the provider reported no usage. */
  tokens?: number;
  /** Tool calls the run made. */
  toolUses: number;
  /** Rounds the run took, one per request to the model. */
  turns: number;
  /** Seconds from the run's first event to its last. */
  seconds: number;
};

/**
 * One line of a diff, as the backend labelled it.
 *
 * `context` lines are kept only to give the changes somewhere to sit, so a long
 * file's diff is a few lines rather than a copy of the file.
 */
export type DiffLine = {
  kind: "context" | "add" | "remove";
  text: string;
};

/**
 * What a write changed, line by line.
 *
 * Produced in Rust while the write tools still hold both versions of the file --
 * the "before" is unrecoverable once the write lands. Stored on the step rather
 * than shown live, because the transcript is the record: a reply that edited
 * three files still has to say what it did to them a week later.
 */
export type ToolDiff = {
  lines: DiffLine[];
  /** Lines left out by the backend's cap, so a truncated diff admits it. */
  hidden?: number;
  /**
   * Added and removed lines over the whole change, before the cap.
   *
   * The authoritative count for the file-changes summary: `lines` is only the
   * part that survived the backend's cap, so counting it reported "+200" for a
   * 500-line creation. Optional because a transcript stored before this existed
   * carries only `lines`, and the count falls back to reading them.
   */
  added?: number;
  removed?: number;
};

/**
 * One stretch of thinking.
 *
 * A model that reasons, calls a tool, then reasons again produces two of these,
 * not one. They are separate because they are separate stretches of time, and
 * merging them would report a duration the user never waited through
 * continuously.
 */
export type ThoughtStep = {
  kind: "thought";
  /** What the model was thinking, as it streamed. */
  text: string;
  /** Seconds spent thinking. Always present once the thought is finished. */
  seconds?: number;
  running: boolean;
  /** Local timestamp when the thought started, for the same reason as above. */
  startedAt?: number;
};

/**
 * Prose the model wrote in the middle of the work rather than as its answer.
 *
 * A reply that thinks, calls a tool, writes a paragraph, calls another tool and
 * only then answers has written two different things: narration and an answer.
 * Both arrive on the stream as ordinary content, so without this the narration
 * was painted into the answer's own bubble and then disappeared when the reply
 * ended -- the stored message keeps the final round only -- which left the user
 * watching text change under them and reading working notes as the conclusion.
 *
 * So a paragraph belongs to the work as soon as a tool call proves the reply was
 * still working. It stays inside "Worked for ...", where the record of the run
 * lives, and the answer's bubble is left to the answer.
 */
export type NoteStep = {
  kind: "note";
  /** What the model said before it carried on. */
  text: string;
  /** Always false in practice: a note is complete the moment it is moved here. */
  running: boolean;
};

export type ChatMessage = {
  role: "user" | "assistant" | "system" | "tool";
  content: string;
  /**
   * Kept in the conversation but not drawn.
   *
   * Used for the turn a finished sub-agent delivers: the model needs the report as
   * input, but the user should not see it as a message of their own -- its result
   * is shown on the `sub_agent` tool row inside the reply that spawned it. It is
   * still persisted and still sent, so a reopened conversation keeps it in context.
   */
  hidden?: boolean;
  reasoning?: string;
  /** The thoughts and tool calls behind this reply, in the order they happened. */
  activity?: ActivityStep[];
  tool?: { name: string; query: string; results: { title: string; url: string; snippet: string }[]; error?: string };
  modelId?: string;
  providerId?: string;
  metrics?: {
    prompt_seconds: number;
    generation_seconds: number;
    /**
     * The whole reply, send to completion.
     *
     * Distinct from the two above because they do not add up to it:
     * `prompt_seconds` stops at the first generated token and
     * `generation_seconds` starts there, so neither is the span the user waited
     * through. The activity panel's "Worked for" figure is this one, because
     * time spent streaming the answer belongs to no step and so cannot be
     * recovered by summing the steps.
     *
     * Optional because a reply stored before this existed has no such figure,
     * and the panel falls back to the step sum for those rather than reporting
     * nothing.
     */
    elapsed_seconds?: number;
    tokens_per_second: number;
    generation_rate_estimated?: boolean;
    completion_tokens?: number;
    prompt_eval_tokens_per_second?: number;
    /**
     * The context at the reply's answer, i.e. its final round only.
     *
     * This is what the context ring and the per-reply charts plot: how full the
     * window was when the answer was written. It is not what the reply spent --
     * see `used_prompt_tokens` for that.
     */
    prompt_tokens?: number;
    /**
     * Every request's input and output for the reply, added up.
     *
     * A reply that ran a tool sends the conversation again for its answer, so the
     * final round's `prompt_tokens` understates the reply whenever a tool ran.
     * Absent on a reply saved before this was recorded, and on one whose provider
     * never reported usage; the session figures fall back to `prompt_tokens` then.
     */
    used_prompt_tokens?: number;
    used_completion_tokens?: number;
  };
};

export type SessionRunStatus = "streaming" | "searching" | "done" | "error";

export type SessionActivity = Record<string, SessionRunStatus>;

export const SESSION_DOT_COLORS = ["#9aa79b", "#e8c48a", "#8fe3a6", "#93ccf5", "#cfaaf2", "#f0d48a", "#f2a49a"] as const;

export const SESSION_DOT_DONE = "#8fe3a6";
export const SESSION_DOT_ERROR = "#f2a49a";
export const SESSION_DOT_DEFAULT = "#9aa79b";

/**
 * Everything about a conversation except its transcript.
 *
 * Split from `ChatSession` so the type system refuses to read `messages` off a
 * header. Transcripts are loaded per conversation rather than all at startup,
 * so "this session has no messages *yet*" is a real state and is different from
 * "this session has no messages" -- one is a load still to happen and the other
 * is an empty conversation. `ChatSession` carries the messages; this does not,
 * so the two cannot be confused at a call site.
 *
 * `promptTokens` / `completionTokens` / `cachedTokens` come from the rollup
 * `save_chat_session` maintains on the `sessions` row, which is why the sidebar
 * and the statistics-adjacent figures do not need the transcript either.
 */
export type ChatSessionHeader = {
  id: string;
  title: string;
  createdAt: string;
  updatedAt: string;
  modelId?: string;
  providerId?: string;
  promptTokens?: number;
  completionTokens?: number;
  cachedTokens?: number | null;
  pinned?: boolean;
  /**
   * True once the title has been set explicitly rather than derived from the
   * opening prompt. Both a manual rename and a model-generated title set it,
   * because either way a later send must not overwrite the title.
   */
  renamed?: boolean;
  /**
   * True only when the user typed the title themselves. This is separate from
   * `renamed` because a generated title also locks the title, but it was not
   * chosen by the user and must not be reported as if it were.
   */
  renamedByUser?: boolean;
  dotColor?: string;
  /**
   * Chat or Agent, for this conversation.
   *
   * Per conversation rather than per window because the two are different kinds
   * of work: a conversation about a codebase wants the agent and its tools, and
   * a one-off question does not. Resetting to Chat on every switch made the mode
   * something to re-pick rather than something chosen.
   *
   * Absent on a conversation saved before this existed, which reads as Chat.
   */
  mode?: "chat" | "agent";
  /**
   * How much Agent mode may do without asking. `undefined` means the stored
   * default rather than a third state, so an unset conversation and one from
   * before this existed behave identically.
   */
  permission?: "ask" | "auto_safe" | "auto_writes" | "full";
  /**
   * What kind of conversation this is: an ordinary chat, or one sub-agent run.
   *
   * Both live in the same table because both are transcripts the same panel can
   * draw. The kind is what keeps them apart: the sidebar lists chats and must not
   * show a run, and the Sub agents panel lists runs and must not show a chat, so
   * each query filters on this rather than one list being asked to show both.
   *
   * `undefined` reads as `chat`, which is what every conversation saved before
   * sub-agents existed is. `job` is a session a scheduled job ran as: listed like a
   * chat, because its transcript is worth reopening, but started by nobody.
   * `telegram` is a conversation answered from a messaging channel; it is listed
   * too, under its own heading, because it is a conversation the user can open
   * and continue from the app.
   */
  kind?: "chat" | "subagent" | "job" | "telegram";
  /**
   * The conversation that spawned this run, present only when `kind` is
   * `subagent`. The panel groups runs under the chat they came from.
   */
  parentSessionId?: string;
};

export type ChatSession = ChatSessionHeader & {
  messages: ChatMessage[];
};
