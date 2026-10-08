import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { listen } from "@tauri-apps/api/event";
import { ArrowUp, Check, Clipboard, LoaderCircle, Paperclip, RotateCcw, SkipForward, Square, X } from "lucide-react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import ActivityPanel from "./ActivityPanel";
import FileChanges from "./FileChanges";
import ModelSwitcher from "../../components/ModelSwitcher";
import ModeSelector from "../../components/ModeSelector";
import SelectionActions, { type SelectionAction } from "./SelectionActions";
import ToolApproval, { type PendingApproval } from "./ToolApproval";
import { userTextOnly } from "../../features/skills/skillMessage";
import { useSessionTitle } from "../../features/models/useSessionTitle";
import {
  contextLengthFor,
  DEFAULT_CONTEXT_LENGTH,
  effortValuesFor,
  loadModelCapabilities,
  reasoningSupported,
  toolsSupported,
  type ModelCapabilities,
} from "../../features/models/modelCapabilities";
import type { ModelSelection } from "../../features/models/types";
import { DEFAULT_CONTEXT } from "../../features/models/localRuntimeDefaults";
import {
  DEFAULT_PERMISSION,
  PERMISSION_SETTING_KEY,
  permissionOrDefault,
  type ChatMode,
  type PermissionMode,
  type WorkingMode,
} from "../../features/chat/chatMode";
import { EMPTY_TITLE_MODELS, type AiEvent, type AttachedFile, type BackgroundRun, type LocalModelEvent, type PendingChat, type SelectedText, type WebSearchOutput } from "./types";
import type { ActivityStep, ChatMessage, ChatSession, SessionRunStatus } from "../../features/chat/types";

import { closeOpenSteps } from "./closeOpenSteps";
import { MARKDOWN_STYLES } from "./markdownStyles";
import { useAttachments } from "./useAttachments";
import ContextRing from "./composer/ContextRing";
import EffortControl from "./composer/EffortControl";
import StartModelDialog from "./composer/StartModelDialog";
import SlashMenu from "./composer/SlashMenu";
import { exactCommand, matchCommands, slashQuery, type SlashCommand } from "./composer/slashCommands";

/**
 * Records a web search's results on the `search_web` step that asked for them.
 *
 * The step is found rather than created because the `tool_call` event for a
 * search has already opened a row by the time results arrive. Results arriving
 * with no open search are appended as their own step rather than dropped: a
 * search that ran and cannot be seen is the one thing worse than an ugly card.
 *
 * Mutates the step in place, like every other handler on `run.activity`, so the
 * panel re-renders from the same array reference the rest of the listener uses.
 */
function attachSearch(run: BackgroundRun, search: WebSearchOutput) {
  const steps = run.activity;
  const open = [...steps]
    .reverse()
    .find((step) => step.kind === "tool" && step.tool === "search_web");
  if (open && open.kind === "tool") {
    open.search = search.results.length > 0 ? search : { query: search.query, results: [] };
  } else {
    steps.push({
      kind: "tool",
      tool: "search_web",
      label: "Search the web",
      detail: search.query,
      outcome: search.results.length > 0 ? `${search.results.length} results` : "No results found",
      failed: false,
      running: false,
      search,
    });
  }
}

type ChatPageProps = {
  endpointId: string;
  model: string;
  localModelId: string;
  session: ChatSession | null;
  /**
   * Whether the open conversation's transcript has arrived.
   *
   * Needed because `session` being null means two different things: nothing is
   * open, or something is open and its messages are still being fetched. Only
   * the first may be composed into. Sending into the second would post a request
   * built from an empty history -- the model would answer as though the
   * conversation had never happened, and the transcript it is based on would
   * never be sent, which is also the one thing that silently destroys the
   * prompt cache for that conversation.
   */
  sessionLoading?: boolean;
  /**
   * Which conversation is open, known even while its transcript is loading.
   *
   * Passed separately from `session` because `session` is null during the load:
   * it is the header joined to the messages, and the messages are exactly what
   * is being fetched. Without this the page would conclude it has no open
   * conversation, drop the id, and tell the backend nothing is on screen -- so a
   * background reply would notify about a conversation the user is watching.
   */
  activeId?: string | null;
  onOpenSettings: () => void;
  /**
   * Returns the conversation to send into, creating it when nothing is open.
   *
   * Carries the mode and access level the send is happening in. The mode selector
   * works before a conversation exists, and a conversation created without the
   * level it was created in reads as Chat -- so an Agent first send would land in
   * a plain chat and the mode would appear to have flipped itself.
   */
  onEnsureSession: (messages: ChatMessage[], workingMode: WorkingMode) => string;
  onFirstSend: () => void;
  onModelSelect: (endpointId: string, model: string) => void;
  onLocalModelSelect: (modelId: string) => void;
  modelRefreshKey: number;
  /** Bumped when the settings modal closes, so chat behaviour settings are re-read. */
  settingsRefreshKey?: number;
  onMessagesChange: (sessionId: string, messages: ChatMessage[]) => void;
  /**
   * Records the conversation's working mode and access level.
   *
   * A header write, so it touches no messages. Kept as a separate callback from
   * `onMessagesChange` because that one carries the transcript, and passing an
   * empty array here would look to the backend like a request to empty the
   * conversation -- which it refuses, correctly.
   */
  onWorkingModeChange: (sessionId: string, workingMode: WorkingMode) => void;
  onActivity: (sessionId: string, status: SessionRunStatus | null) => void;
  onNotify: (tone: "success" | "error", message: string) => void;
  onOpenUrl: (url: string) => void;
  /**
   * Opens a workspace-relative file in the right panel.
   *
   * A single callback rather than the panel's own state, so the transcript never
   * holds a reference to the panel and the panel never has to know a transcript
   * exists. The paths come from the tools, which refuse absolute paths, so they
   * are already in the form the panel's read command takes.
   */
  onOpenFile?: (path: string) => void;
  /** Models that name new conversations, in order; empty disables the feature. */
  titleModels?: ModelSelection[];
  onAutoTitle: (sessionId: string, title: string) => void;
};

export default function ChatPage({ endpointId, model, localModelId, session, sessionLoading = false, activeId = null, onOpenSettings, onEnsureSession, onFirstSend, onActivity, onModelSelect, onLocalModelSelect, modelRefreshKey, settingsRefreshKey = 0, onMessagesChange, onWorkingModeChange, onNotify, onOpenUrl, onOpenFile, titleModels, onAutoTitle }: ChatPageProps) {
  const [messages, setMessages] = useState<ChatMessage[]>(session?.messages ?? []);
  // The latest transcript, for the run listener (a mount-time effect) to build on
  // when something arrives outside the current render -- a background sub-agent's
  // report is dispatched from there.
  const messagesRef = useRef<ChatMessage[]>(messages);
  messagesRef.current = messages;
  const [input, setInput] = useState("");
  const [draggingFiles, setDraggingFiles] = useState(false);
  const [running, setRunning] = useState(false);
  const [streamText, setStreamText] = useState("");
  const [streamReasoning, setStreamReasoning] = useState("");
  // The steps behind the reply now streaming, in the order they happened. Held
  // apart from `streamReasoning` because a stretch of thinking and a tool call
  // are different things in the panel: one is a block of text with a duration,
  // the other a row that can be running, cached or failed.
  const [streamActivity, setStreamActivity] = useState<ActivityStep[]>([]);
  const [searching, setSearching] = useState(false);
  const [effort, setEffort] = useState("medium");
  // Agent mode and its permission level, both belonging to the open conversation.
//
// Stored on the session rather than held only in state, so switching away and
// back -- or moving to another page and returning -- restores the mode the
// conversation was actually being worked in. It was per page before, so every
// switch reset it to Chat and Agent had to be picked again for each conversation.
//
// The app-wide default is still read once: a conversation that has never chosen
// a level uses it, so the setting is a floor rather than something each
// conversation has to opt into.
const [mode, setMode] = useState<ChatMode>(session?.mode ?? "chat");
  const [permission, setPermission] = useState<PermissionMode>(session?.permission ?? DEFAULT_PERMISSION);
  /**
   * The tool calls the agent is blocked on, oldest first.
   *
   * A queue rather than a single slot, because more than one can be waiting at
   * once: an agent that spawns several sub-agents in a turn gives each of them its
   * own tools, and in `Ask` mode two of them can be sitting on a prompt at the
   * same moment. A single slot would show the second and silently strand the
   * first, which the run cannot recover from — it is parked until answered or
   * cancelled. The oldest is shown; the rest follow as they are answered.
   */
  const [pendingApprovals, setPendingApprovals] = useState<PendingApproval[]>([]);
  const [contextLimit, setContextLimit] = useState<number | null>(null);
  // What the selected model's provider says it supports. Null until it has been
  // read, and every field inside it may still be unknown.
  const [capabilities, setCapabilities] = useState<ModelCapabilities | null>(null);
  const {
    attachments,
    add: addAttachments,
    addPaths,
    addBrowserFiles,
    chooseFiles,
    remove: removeAttachment,
    removeSent: removeSentAttachments,
    restore: restoreAttachments,
  } = useAttachments({
    // Passed as a value rather than read from a ref: what a file may be depends
    // on the model in hand, so a new model's capabilities have to reach the hook
    // on the same render that changes the picker.
    capabilities,
    onNotify,
  });
  const [copiedIndex, setCopiedIndex] = useState<number | null>(null);
  // Web search is on by default for every conversation. The composer's `+` menu
  // is gone; the `/web-on` and `/web-off` slash commands are how it is changed.
  const [webSearchEnabled, setWebSearchEnabled] = useState(true);
  // Which row of the slash menu is highlighted. Clamped against the matches at
  // the point of use, so a shrinking list can never point past its end.
  const [slashIndex, setSlashIndex] = useState(0);
  // Messages typed while a reply was already running, sent the moment it ends.
  // Held in a ref because the run listener (a mount-time effect) drains it, and
  // mirrored in a count so the composer can show how many are waiting.
  const queuedMessagesRef = useRef<
    { sessionId: string; text: string; attachments: AttachedFile[]; hidden?: boolean }[]
  >([]);
  const [queuedCount, setQueuedCount] = useState(0);
  // `dispatch` is rebuilt every render; the run listener holds this ref so it
  // calls the latest one rather than a stale closure from mount.
  const dispatchRef = useRef<
    (
      messageText: string,
      attachments: AttachedFile[],
      baseMessages: ChatMessage[],
      sessionId: string | null,
      clearDraft?: boolean,
      hidden?: boolean,
    ) => Promise<void>
  >(async () => {});
  const [startModelPrompt, setStartModelPrompt] = useState(false);
  const [startingModel, setStartingModel] = useState(false);
  const pendingChatRef = useRef<PendingChat | null>(null);
  /**
   * Mode and access level per conversation, as last chosen in this window.
   *
   * Ahead of the stored values because a switch can be made before the next save
   * lands, and because the transcript being loaded leaves `session` null for a
   * moment. Bounded by conversation count, which is already in memory as headers.
   */
  const workingModesRef = useRef<Map<string, { mode: ChatMode; permission: PermissionMode }>>(new Map());
  const threadRef = useRef<HTMLDivElement>(null);
  const autoFollowRef = useRef(true);
  const runIdRef = useRef("");
  const requestStartedAtRef = useRef(0);
  const generationStartedAtRef = useRef(0);
  const sessionIdRef = useRef<string | null>(session?.id ?? null);
  const [selection, setSelection] = useState<SelectedText | null>(null);
  // Read once on mount from the same SQLite records Settings writes, so the composer
  // matches the preferences without the settings modal having to push them down.
  const [showMetrics, setShowMetrics] = useState(true);
  const [sendKey, setSendKey] = useState<"Enter" | "Ctrl + Enter">("Enter");
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const backgroundRunsRef = useRef<Map<string, BackgroundRun>>(new Map());
  const sessionRunIdsRef = useRef<Map<string, string>>(new Map());
  const streamTextRef = useRef("");
  const streamReasoningRef = useRef("");
  const listenerReadyRef = useRef<Promise<void>>(Promise.resolve());
  const onActivityRef = useRef(onActivity);
  onActivityRef.current = onActivity;
  const onMessagesChangeRef = useRef(onMessagesChange);
  onMessagesChangeRef.current = onMessagesChange;
  const onWorkingModeChangeRef = useRef(onWorkingModeChange);
  onWorkingModeChangeRef.current = onWorkingModeChange;
  // Read inside `send`, which is re-created on every render but must not be
  // rebuilt around the value: a ref keeps the guard reading the current state
  // without making the handler a dependency of the key handler.
  const sessionLoadingRef = useRef(sessionLoading);
  sessionLoadingRef.current = sessionLoading;
  // Names the conversation once, after the first assistant reply. Held in a ref
  // because the event listener is registered once and would otherwise capture a
  // stale selection when the user changes the title models in Settings.
  const titleSession = useSessionTitle({
    selections: titleModels ?? EMPTY_TITLE_MODELS,
    onTitle: onAutoTitle,
    // Naming a conversation is a background nicety. If every configured model
    // fails, the chat is still fine and the title simply stays as the opening
    // message, so the failure is logged rather than put in front of the user as
    // a notification they cannot act on.
    onError: (message) => console.warn("Title generation failed:", message),
  });
  const titleSessionRef = useRef(titleSession);
  titleSessionRef.current = titleSession;

  // One markdown configuration for the answer, a thought and a selection draft,
  // so a thought renders with the same fences and lists the answer does. A model
  // writes its thinking the way it writes an answer, and formatting one and not
  // the other makes the thought look like something the app did to it.
  const markdownComponents = useMemo(() => ({
    a: ({ href, children, ...props }: React.ComponentProps<"a">) => (
      <a
        {...props}
        href={href}
        onClick={(event) => {
          event.preventDefault();
          if (href) onOpenUrl(href);
        }}
        onAuxClick={(event) => {
          event.preventDefault();
          if (href) onOpenUrl(href);
        }}
      >
        {children}
      </a>
    ),
  }), [onOpenUrl]);
  const renderMarkdown = (text: string) => <ReactMarkdown remarkPlugins={[remarkGfm]} components={markdownComponents}>{text}</ReactMarkdown>;

  useEffect(() => {
    // `activeId` rather than `session?.id`: the two disagree exactly while a
    // transcript is loading, and that is the window in which dropping the id
    // would misreport the on-screen conversation to the backend.
    const nextId = activeId ?? session?.id ?? null;
    sessionIdRef.current = nextId;
    // Cleared only when a conversation is genuinely open or genuinely closed.
    // During the fetch `session` is null while `nextId` is set, and clearing here
    // would empty the transcript for as long as the load takes -- showing the
    // user a conversation that has lost its history and then repopulating it.
    if (session || !sessionLoading) setMessages(session?.messages ?? []);
    autoFollowRef.current = true;
    // Each conversation carries the mode and access level it was last worked in,
    // so switching restores them rather than falling back to Chat.
    //
    // Read from the remembered pair when there is one, and from the session
    // itself otherwise. The remembered pair is what covers the window where a
    // conversation is selected but its transcript has not arrived: `session` is
    // null then, and resetting to Chat for the length of a load would show the
    // wrong mode on the way back in. With nothing open, the defaults return --
    // a new conversation starts in Chat.
    if (nextId) {
      const remembered = workingModesRef.current.get(nextId);
      if (remembered) {
        setMode(remembered.mode);
        setPermission(remembered.permission);
      } else {
        setMode(session?.mode ?? "chat");
        setPermission(session?.permission ?? DEFAULT_PERMISSION);
      }
    } else {
      setMode("chat");
      setPermission(DEFAULT_PERMISSION);
    }
    // Tells the backend which session is on screen, so a reply finishing here does not
    // raise a notification for something the user is already watching.
    void invoke("ai_set_viewed_session", { sessionId: nextId }).catch(() => undefined);
    const runId = (nextId && sessionRunIdsRef.current.get(nextId)) || "";
    runIdRef.current = runId;
    const run = (runId && backgroundRunsRef.current.get(runId)) || null;
    streamTextRef.current = run?.text ?? "";
    streamReasoningRef.current = run?.reasoning ?? "";
    setStreamText(run?.text ?? "");
    setStreamReasoning(run?.reasoning ?? "");
    // Restored from the run rather than kept in state alone: a reply that kept
    // going while the user read another session has to come back with the steps
    // it took, or switching away would silently drop the record of them.
    setStreamActivity(run?.activity ?? []);
    setSearching(run?.searching === true);
    setRunning(Boolean(run));
  }, [activeId, session?.id, session?.messages, sessionLoading]);

  useEffect(() => {
    let active = true;
    invoke<{ showMetrics?: boolean; sendWith?: "Enter" | "Ctrl + Enter" } | null>("database_get_setting", { key: "app.general" })
      .then((general) => {
        if (!active || !general) return;
        if (general.showMetrics !== undefined) setShowMetrics(general.showMetrics);
        if (general.sendWith) setSendKey(general.sendWith);
      })
      .catch(() => undefined);
    return () => { active = false; };
  }, [settingsRefreshKey]);

  useEffect(() => {
    if (!localModelId) {
      setContextLimit(null);
      return;
    }
    let active = true;
    invoke<{ context: number }>("local_runtime_settings", { modelId: localModelId }).then((settings) => {
      if (active) setContextLimit(settings.context);
    }).catch(() => {
      // The composer ring needs a figure to draw against even when the settings
      // read failed. The same default the backend would have used, so the ring
      // does not briefly claim a different limit from the one the model is
      // actually about to run at.
      if (active) setContextLimit(DEFAULT_CONTEXT);
    });
    return () => { active = false; };
  }, [localModelId]);

  // Learn what the selected remote model supports. A local model keeps its own
  // context setting above and has no capabilities to read.
  useEffect(() => {
    if (!endpointId || !model || localModelId) {
      setCapabilities(null);
      return;
    }
    let active = true;
    void loadModelCapabilities(endpointId, model).then((result) => {
      if (active) setCapabilities(result);
    });
    return () => { active = false; };
  }, [endpointId, model, localModelId, modelRefreshKey]);

  // Prefer the provider's real prompt token count from the latest reply, and add the
  // text generated since then. Fall back to a character estimate when usage is unknown.
  const lastReportedPromptTokens = [...messages].reverse().find((message) => message.metrics?.prompt_tokens !== undefined)?.metrics?.prompt_tokens ?? 0;
  const textSinceLastReport = lastReportedPromptTokens === 0
    ? messages.reduce((total, message) => total + message.content.length + (message.tool?.results.reduce((sum, result) => sum + result.title.length + result.snippet.length, 0) ?? 0), 0) + streamText.length + streamReasoning.length
    : streamText.length + streamReasoning.length;
  const contextIsEstimated = lastReportedPromptTokens === 0;
  const contextTokens = contextIsEstimated
    ? Math.ceil(textSinceLastReport / 4)
    : lastReportedPromptTokens + Math.ceil(textSinceLastReport / 4);
  // A local model keeps the context size set in its runtime settings; a remote
  // one uses whatever its provider reported, falling back to a conservative
  // default rather than the generous guess this used to assume.
  const remoteContext = contextLengthFor(capabilities);
  const displayedContextLimit = contextLimit ?? remoteContext ?? DEFAULT_CONTEXT_LENGTH;
  // The effort levels are whatever this model accepts, in its own provider's
  // vocabulary. Keep the current choice only if the new model still offers it.
  const effortValues = effortValuesFor(capabilities);
  useEffect(() => {
    setEffort((current) => (effortValues.includes(current) ? current : "medium"));
  }, [effortValues.join(",")]);
  const canSetEffort = reasoningSupported(capabilities);
  const canUseTools = toolsSupported(capabilities);
  // The stored level is a default for conversations that have never chosen one,
  // not an override. It is read once and applied only where the open conversation
  // has no level of its own, so switching to a conversation that chose one does
  // not replace it -- which is the same failure the per-conversation mode had.
  useEffect(() => {
    void (async () => {
      try {
        const stored = await invoke<string | null>("database_get_setting", { key: PERMISSION_SETTING_KEY });
        const fallback = permissionOrDefault(stored);
        setPermission((current) => (current === DEFAULT_PERMISSION ? fallback : current));
      } catch {
        // No stored value, or a database that predates the setting. The default
        // is already the safe one, so nothing to do.
      }
    })();
  }, []);

  /**
   * Records the mode and access level against the open conversation.
   *
   * Both are remembered in the window immediately and written to the session
   * alongside it. The write is a header write, so it carries no messages and
   * cannot truncate a transcript -- changing the mode mid-conversation must not
   * be able to cost the conversation its history.
   *
   * The backend reads `app.agent_permission` per run, so that setting is kept
   * pointing at whatever was last chosen. A new conversation then opens with the
   * level the user last worked at, which is the behaviour that had, while each
   * conversation still remembers its own.
   */
  const rememberWorkingMode = useCallback((nextMode: ChatMode, nextPermission: PermissionMode) => {
    const id = sessionIdRef.current;
    if (!id) return;
    workingModesRef.current.set(id, { mode: nextMode, permission: nextPermission });
    onWorkingModeChangeRef.current(id, { mode: nextMode, permission: nextPermission });
  }, []);

  const changeMode = (next: ChatMode) => {
    setMode(next);
    rememberWorkingMode(next, permission);
  };

  const changePermission = (next: PermissionMode) => {
    setPermission(next);
    rememberWorkingMode(mode, next);
    void invoke("database_set_setting", { key: PERMISSION_SETTING_KEY, value: next }).catch(() => {
      // A failed write is not worth interrupting the user for; the level still
      // applies to the next reply, and it is read per run on the backend.
    });
  };
  const persistMessages = (sessionId: string | null, next: ChatMessage[]) => {
    if (sessionId) onMessagesChange(sessionId, next);
  };

  /**
   * Removes one answered prompt, leaving any others a concurrent agent raised.
   *
   * By id rather than by clearing the list, so answering one of two stacked
   * prompts does not dismiss the other before the user has seen it.
   */
  const dismissApproval = (approvalId: string) => {
    setPendingApprovals((current) => current.filter((approval) => approval.approvalId !== approvalId));
  };

  useEffect(() => {
    const unlistenPromise = getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type === "enter" || event.payload.type === "over") setDraggingFiles(true);
      if (event.payload.type === "drop") {
        setDraggingFiles(false);
        void addPaths(event.payload.paths);
      }
      if (event.payload.type === "leave") setDraggingFiles(false);
    });
    return () => { unlistenPromise.then((unlisten) => unlisten()); };
  }, []);

  useEffect(() => {
    const unlistenPromise = listen<AiEvent>("ai-event", (event) => {
      const payload = event.payload;
      // Puts a finished sub-agent's report on the `sub_agent` row that started it,
      // so it reads as the tool's own output inside "Worked for ...". The row lives
      // in the live run's activity while the reply is still going and in a stored
      // assistant message once it has ended, so both are patched -- and the live one
      // matters because its activity is what gets persisted when the reply ends.
      const attachSubagentReport = (sessionId: string, report: string) => {
        const fill = (steps: ActivityStep[]): boolean => {
          for (let index = steps.length - 1; index >= 0; index--) {
            const step = steps[index];
            if (step.kind === "tool" && step.tool === "sub_agent" && step.output === undefined) {
              step.output = report;
              step.outcome = "finished";
              return true;
            }
          }
          return false;
        };

        const runId = sessionRunIdsRef.current.get(sessionId);
        const live = runId ? backgroundRunsRef.current.get(runId) : undefined;
        if (live && fill(live.activity)) setStreamActivity([...live.activity]);

        const next = [...messagesRef.current];
        for (let index = next.length - 1; index >= 0; index--) {
          const message = next[index];
          if (message.role !== "assistant" || !message.activity) continue;
          const activity = message.activity.map((step) => ({ ...step }));
          if (!fill(activity)) continue;
          next[index] = { ...message, activity };
          messagesRef.current = next;
          setMessages(next);
          onMessagesChangeRef.current(sessionId, next);
          return;
        }
      };
      // A background sub-agent's report. It arrives under the agent's own run id,
      // which is not a chat run, so it is handled before the run lookup below.
      // The report becomes a user turn so the model picks it up and can act.
      if (payload.kind === "subagent_result") {
        const reportSession = payload.metrics?.session_id;
        const report = payload.text;
        if (reportSession && report && reportSession === sessionIdRef.current) {
          const label = payload.metrics?.label ?? "agent";
          // The report goes on the `sub_agent` row inside "Worked for ...", not into
          // the transcript as the user's own message; the model still receives it, as
          // a hidden turn (below).
          attachSubagentReport(reportSession, report);
          const content = `Sub-agent \`${label}\` finished and reported:\n\n${report}`;
          if (sessionRunIdsRef.current.has(reportSession)) {
            // Still answering: deliver it the moment this reply ends.
            queuedMessagesRef.current.push({ sessionId: reportSession, text: content, attachments: [], hidden: true });
            setQueuedCount(queuedMessagesRef.current.length);
          } else {
            // The turn is over, so the report starts its own. `clearDraft` is false
            // because this is not the user's message -- it must not wipe a draft.
            void dispatchRef.current(content, [], messagesRef.current, reportSession, false, true);
          }
        }
        return;
      }
      // An approval a background sub-agent raised. Handled before the run lookup
      // because the run that owns the screen may already have ended -- the prompt
      // still has to reach the user, or the agent could never get a call approved.
      if (payload.kind === "tool_approval" && payload.metrics?.approval_id) {
        const approval: PendingApproval = {
          runId: payload.run_id,
          approvalId: payload.metrics.approval_id,
          tool: payload.metrics.tool ?? "tool",
          command: payload.metrics.command ?? "",
          arguments: payload.metrics.arguments ?? {},
        };
        // Guarded against a repeat of the same id, which a re-emitted event would
        // otherwise queue twice and then answer with one click.
        setPendingApprovals((current) => current.some((entry) => entry.approvalId === approval.approvalId) ? current : [...current, approval]);
        return;
      }
      const run = backgroundRunsRef.current.get(payload.run_id);
      if (!run) return;
      const isActive = run.sessionId === sessionIdRef.current;
      const finishRun = () => {
        run.activity = closeOpenSteps(run.activity);
        backgroundRunsRef.current.delete(payload.run_id);
        if (sessionRunIdsRef.current.get(run.sessionId) === payload.run_id) {
          sessionRunIdsRef.current.delete(run.sessionId);
        }
      };
      // A message typed while this conversation was answering. Sent now that the
      // reply has ended, so the queue drains in order and one reply never starts
      // on top of another. Matched by session, so a message queued for one
      // conversation is not sent into another.
      const drainQueued = (sessionId: string, base: ChatMessage[]) => {
        const index = queuedMessagesRef.current.findIndex((item) => item.sessionId === sessionId);
        if (index === -1) return;
        const [item] = queuedMessagesRef.current.splice(index, 1);
        setQueuedCount(queuedMessagesRef.current.length);
        void dispatchRef.current(item.text, item.attachments, base, sessionId, false, item.hidden ?? false);
      };
      if (payload.kind === "completion_id" && payload.text) {
        run.completionId = payload.text;
      } else if (payload.kind === "reasoning_phase") {
        run.isReasoning = payload.text === "reasoning";
      } else if (payload.kind === "status") {
        // Set once, on the first `thinking` of a run. Every later round emits the
        // same status, and overwriting on each one restarted the clock the
        // activity panel measures against -- so "Worked for" reported the last
        // round rather than the whole run, and a reply that spent thirty seconds
        // across four tool calls said five.
        //
        // No clearing needed: `finishRun` deletes the run from the map, so the
        // next run is a fresh object starting at zero.
        if (payload.text === "thinking" && !run.requestStartedAt) {
          run.requestStartedAt = performance.now();
        }
        run.searching = payload.text === "searching";
        if (isActive) {
          requestStartedAtRef.current = run.requestStartedAt;
          setSearching(run.searching);
        }
        onActivityRef.current(run.sessionId, run.searching ? "searching" : "streaming");
      } else if (payload.kind === "web_search" && payload.search) {
        // Attached to the `search_web` step rather than stored as its own message.
        // As a message it rendered as a card floating above the reply it belongs
        // to, outside the panel that holds every other tool; as step data it opens
        // in place, behind a disclosure, alongside the call that produced it.
        attachSearch(run, payload.search);
      } else if (payload.kind === "web_search_failed") {
        attachSearch(run, { query: payload.text ?? "", results: [] });
      } else if (payload.kind === "reasoning_delta" && payload.text) {
        // Appended to the last thought rather than kept as one growing string,
        // so a thought that started before a tool call stays its own step
        // instead of merging with the thinking that came after it.
        const steps = run.activity;
        const last = steps[steps.length - 1];
        if (last?.kind === "thought" && last.running) {
          last.text += payload.text;
        } else {
          steps.push({ kind: "thought", text: payload.text, running: true, startedAt: Date.now() });
        }
        run.reasoning += payload.text;
        if (isActive) {
          streamReasoningRef.current = run.reasoning;
          setStreamReasoning(run.reasoning);
          setStreamActivity([...steps]);
        }
      } else if (payload.kind === "reasoning_segment" && payload.text) {
        // The backend closed a stretch of thinking, which is the only moment its
        // duration is knowable. The text arrives whole rather than as deltas,
        // so it replaces whatever streamed in place of it.
        const steps = run.activity;
        const last = steps[steps.length - 1];
        const seconds = last?.kind === "thought" && last.startedAt !== undefined
          ? (Date.now() - last.startedAt) / 1000
          : 0;
        const segment: ActivityStep = { kind: "thought", text: payload.text, seconds, running: false };
        if (last?.kind === "thought" && last.running) steps[steps.length - 1] = segment;
        else steps.push(segment);
        if (isActive) setStreamActivity([...steps]);
      } else if (payload.kind === "tool_call" && payload.metrics) {
        // A call the model has asked for but not yet run. The row appears now so
        // a slow tool is visibly in flight rather than absent until it returns.
        const steps = run.activity;
        steps.push({
          kind: "tool",
          tool: payload.text ?? "tool",
          label: payload.metrics.label ?? "Tool call",
          detail: payload.metrics.detail ?? "",
          outcome: "",
          failed: false,
          running: true,
          startedAt: Date.now(),
        });
        if (isActive) setStreamActivity([...steps]);
      } else if (payload.kind === "tool_result" && payload.metrics) {
        // Closes the row the matching `tool_call` opened. Matched on the tool
        // name rather than the index: indices restart every round, and two
        // rounds can both contain one `read_file`, so an index alone would let a
        // result land on a row from a round that had already closed.
        const steps = run.activity;
        const metrics = payload.metrics;
        const open = [...steps].reverse().find((step) => step.kind === "tool" && step.running && step.tool === (payload.text ?? "tool"));
        if (open && open.kind === "tool") {
          open.running = false;
          open.seconds = metrics.seconds;
          open.outcome = metrics.outcome ?? "";
          open.failed = metrics.failed === true;
          if (metrics.cached) open.cached = true;
          // What the model was handed, so opening the row shows the real result
          // rather than the one-line summary beside it.
          if (metrics.output) open.output = metrics.output;
          // A write's diff, which the panel draws in place of the raw text.
          if (metrics.diff) open.diff = metrics.diff;
          // Carried onto the row so a created file is distinguishable from a
          // rewritten one. Both look the same in the diff — every line added,
          // none removed — so this is the only thing that tells them apart.
          if (metrics.created) open.created = true;
          // `startedAt` is dropped once the call is done: the row is reporting
          // the backend's measured figure now, and leaving a local timestamp on
          // it would let a later tick add to a call that has already finished.
          delete open.startedAt;
        } else {
          // A result with no open row. Reported rather than dropped, because a
          // call the user cannot see is one they cannot account for.
          steps.push({
            kind: "tool",
            tool: payload.text ?? "tool",
            label: metrics.label ?? "Tool call",
            detail: metrics.detail ?? "",
            outcome: metrics.outcome ?? "",
            failed: metrics.failed === true,
            seconds: metrics.seconds,
            running: false,
            ...(metrics.cached ? { cached: true } : {}),
            ...(metrics.output ? { output: metrics.output } : {}),
            ...(metrics.diff ? { diff: metrics.diff } : {}),
            ...(metrics.created ? { created: true } : {}),
          });
        }
        if (isActive) setStreamActivity([...steps]);
      } else if (payload.kind === "delta" && payload.text) {
        if (!run.generationStartedAt) run.generationStartedAt = performance.now();
        run.text += payload.text;
        if (isActive) {
          generationStartedAtRef.current = run.generationStartedAt;
          streamTextRef.current = run.text;
          setStreamText(run.text);
        }
      } else if (payload.kind === "completed") {
        const completedAt = performance.now();
        // Falls back to now only if no `thinking` status ever arrived, which
        // would mean the backend failed before it began. Any real round sets it,
        // and it is set once -- see the note on `requestStartedAt`.
        const startedAt = run.requestStartedAt || completedAt;
        const promptSeconds = Math.max(0, (run.generationStartedAt || completedAt) - startedAt) / 1000;
        const generationSeconds = Math.max(0.001, (run.generationStartedAt ? completedAt - run.generationStartedAt : completedAt - startedAt) / 1000);
        // The whole reply, which is neither figure above: `promptSeconds` stops
        // at the first generated token and `generationSeconds` starts there, so
        // between them they cover the run but neither is the answer to "how long
        // did I wait". The panel needs the total, and this is the only place that
        // knows both ends of it.
        const elapsedSeconds = Math.max(0, completedAt - startedAt) / 1000;
        const response = payload.text || run.text;
        const estimatedTokens = Math.max(1, Math.round(response.length / 4));
        const completionTokens = payload.metrics?.completion_tokens ?? estimatedTokens;
        const generationTokensPerSecond = payload.metrics?.generation_tokens_per_second ?? completionTokens / generationSeconds;
        const promptTokens = payload.metrics?.prompt_tokens;
        const promptEvalTokensPerSecond = payload.metrics?.prompt_eval_tokens_per_second;
        const next = [...run.baseMessages, { role: "assistant" as const, content: response, reasoning: run.reasoning || undefined, ...(run.activity.length > 0 ? { activity: run.activity } : {}), modelId: run.modelLabel, providerId: run.providerLabel, metrics: { prompt_seconds: promptSeconds, generation_seconds: generationSeconds, elapsed_seconds: elapsedSeconds, tokens_per_second: generationTokensPerSecond, generation_rate_estimated: payload.metrics?.generation_tokens_per_second == null, completion_tokens: completionTokens, ...(promptTokens !== undefined ? { prompt_tokens: promptTokens } : {}), ...(promptEvalTokensPerSecond !== undefined ? { prompt_eval_tokens_per_second: promptEvalTokensPerSecond } : {}) } }];
        finishRun();
        onActivityRef.current(run.sessionId, isActive ? null : "done");
        onMessagesChangeRef.current(run.sessionId, next);
        if (isActive) {
          streamTextRef.current = "";
          streamReasoningRef.current = "";
          setStreamText("");
          setStreamReasoning("");
          setStreamActivity([]);
          setSearching(false);
          setRunning(false);
          setMessages(next);
        }
        drainQueued(run.sessionId, next);
      } else if (payload.kind === "stopped") {
        // `finishRun` runs here as it does on every other terminating event, even
        // though `stop()` has usually already removed the run and this listener
        // therefore sees nothing. That is the point: `stop()` covers the stop the
        // user pressed, and this covers a stop the backend raised on its own —
        // a cancellation from elsewhere, a timeout, a second window. Without it
        // that run stayed in both maps forever and the send guards refused every
        // further reply in that conversation.
        //
        // `run` is a local reference, so it is still readable after `finishRun`
        // has taken the run out of the maps — and reading it afterwards is what
        // gets the closed steps, since `finishRun` is what closes them.
        finishRun();
        onActivityRef.current(run.sessionId, null);
        let base: ChatMessage[] = run.baseMessages;
        if (run.text) {
          // Whatever arrived is kept, for the reason `stop()` keeps it: the
          // transcript is the record, and a cancelled reply that dropped its
          // answer misreports what happened.
          base = [...run.baseMessages, { role: "assistant" as const, content: run.text, reasoning: run.reasoning || undefined, ...(run.activity.length > 0 ? { activity: run.activity } : {}), modelId: run.modelLabel, providerId: run.providerLabel }];
          onMessagesChangeRef.current(run.sessionId, base);
          if (isActive) setMessages(base);
        }
        if (isActive) {
          setPendingApprovals([]);
          setSearching(false);
          setRunning(false);
          streamTextRef.current = "";
          streamReasoningRef.current = "";
          setStreamText("");
          setStreamReasoning("");
          setStreamActivity([]);
        }
        drainQueued(run.sessionId, base);
      } else if (payload.kind === "failed") {
        const message = payload.error || payload.text || "The provider could not complete this response.";
        if (isActive) onNotify("error", message);
        run.text = run.text ? `${run.text}\n\nRequest failed: ${message}` : `Request failed: ${message}`;
        const next = [...run.baseMessages, { role: "assistant" as const, content: run.text, reasoning: run.reasoning || undefined, ...(run.activity.length > 0 ? { activity: run.activity } : {}), modelId: run.modelLabel, providerId: run.providerLabel }];
        finishRun();
        onActivityRef.current(run.sessionId, isActive ? null : "error");
        onMessagesChangeRef.current(run.sessionId, next);
        if (isActive) {
          streamTextRef.current = "";
          streamReasoningRef.current = "";
          setStreamText("");
          setStreamReasoning("");
          setStreamActivity([]);
          setSearching(false);
          setRunning(false);
          setMessages(next);
        }
        drainQueued(run.sessionId, next);
      }
    });
    listenerReadyRef.current = unlistenPromise.then(() => undefined);
    return () => {
      // Every run still in flight is written out before the listener goes away.
      //
      // Unlistening is what loses a reply: the backend emits `completed` for a
      // run this listener is no longer receiving, so the assistant message is
      // never built and the partial answer is never stored. `ChatPage` unmounts
      // whenever the user opens Statistics or Database — the header keeps the
      // frame and only the page leaves — so this is a routine click, not a
      // shutdown path.
      //
      // The run is not cancelled. A background reply keeps streaming and will
      // notify when it lands, exactly as one started in another conversation
      // does; only the record of it is written here. Cancelling would throw away
      // work the user paid for and never asked to stop.
      for (const [runId, run] of backgroundRunsRef.current) {
        backgroundRunsRef.current.delete(runId);
        if (run.text) {
          onMessagesChangeRef.current(run.sessionId, [...run.baseMessages, { role: "assistant" as const, content: run.text, reasoning: run.reasoning || undefined, ...(run.activity.length > 0 ? { activity: closeOpenSteps(run.activity) } : {}), modelId: run.modelLabel, providerId: run.providerLabel }]);
        }
        onActivityRef.current(run.sessionId, null);
      }
      sessionRunIdsRef.current.clear();
      unlistenPromise.then((unlisten) => unlisten());
    };
  }, [onNotify]);

  useEffect(() => {
    const thread = threadRef.current;
    if (!thread) return;
    if (autoFollowRef.current) thread.scrollTop = thread.scrollHeight;
  }, [messages, streamText, streamReasoning]);

  const handleThreadScroll = () => {
    const thread = threadRef.current;
    if (!thread) return;
    autoFollowRef.current = thread.scrollHeight - thread.scrollTop - thread.clientHeight < 48;
  };

  const sendMessages = async (requestMessages: ChatMessage[], sessionId: string, requestAttachments: AttachedFile[], skillIds: string[], clearDraft = true) => {
    if (sessionRunIdsRef.current.has(sessionId) || (!endpointId && !localModelId)) return false;
    sessionIdRef.current = sessionId;
    autoFollowRef.current = true;
    const runId = crypto.randomUUID();
    const modelLabel = localModelId || model;
    const providerLabel = endpointId || undefined;
    const sessionTitle = session?.id === sessionId ? session?.title ?? "" : "";
    backgroundRunsRef.current.set(runId, {
      sessionId,
      baseMessages: requestMessages,
      text: "",
      reasoning: "",
      activity: [],
      searching: false,
      modelLabel,
      providerLabel,
      requestStartedAt: 0,
      generationStartedAt: 0,
      localModel: Boolean(localModelId),
      isReasoning: false,
    });
    sessionRunIdsRef.current.set(sessionId, runId);
    onActivityRef.current(sessionId, sessionId === sessionIdRef.current ? null : "streaming");
    // Cleared, not seeded. The run stamps this itself on the first `thinking`
    // status, which is the moment the backend actually began; seeding it here
    // would either be overwritten anyway or start the clock before the request
    // was made.
    requestStartedAtRef.current = 0;
    generationStartedAtRef.current = 0;
    streamTextRef.current = "";
    streamReasoningRef.current = "";
    setStreamText("");
    setStreamReasoning("");
    setStreamActivity([]);
    setSearching(false);
    setRunning(true);
    // A queued message is sent from a draft the user already cleared, so the field
    // is left alone -- clearing it again would wipe whatever they have typed since.
    if (clearDraft) {
      setInput("");
      removeSentAttachments(requestAttachments);
    }
    runIdRef.current = runId;
    setMessages(requestMessages);
    onMessagesChange(sessionId, requestMessages);

    await listenerReadyRef.current;
    let instructions = "";
    try {
      const general = await invoke<{ instructions?: string } | null>("database_get_setting", { key: "app.general" });
      instructions = general?.instructions?.trim() ?? "";
    } catch {
      instructions = "";
    }
    const messagesWithInstructions = instructions
      ? [{ role: "system", content: instructions }, ...requestMessages]
      : requestMessages;
    try {
      await invoke("ai_chat", {
        request: { run_id: runId, session_id: sessionId, session_title: sessionTitle || undefined, messages: messagesWithInstructions, endpoint_id: endpointId || null, local_model: Boolean(localModelId), model: localModelId ? "projectz-local" : model || null, // Sent verbatim: the value came from the provider's own list of accepted
        // levels, so translating or guessing it here is what causes HTTP 400.
        reasoning: canSetEffort ? effort : undefined, web_search_enabled: webSearchEnabled, mode, permission, attachments: requestAttachments.map(({ name, mime_type, data_base64 }) => ({ name, mime_type, data_base64 })), skill_ids: skillIds },
      });
      return true;
    } catch (reason: unknown) {
      const message = String(reason);
      backgroundRunsRef.current.delete(runId);
      if (sessionRunIdsRef.current.get(sessionId) === runId) sessionRunIdsRef.current.delete(sessionId);
      onActivityRef.current(sessionId, sessionId === sessionIdRef.current ? null : "error");
      onNotify("error", message);
      const next = [...requestMessages, { role: "assistant" as const, content: `Request failed: ${message}` }];
      persistMessages(sessionId, next);
      if (sessionId === sessionIdRef.current) {
        setMessages(next);
        setStreamText("");
        setStreamReasoning("");
        streamReasoningRef.current = "";
        streamTextRef.current = "";
        setSearching(false);
        setRunning(false);
      }
      if (clearDraft) restoreAttachments(requestAttachments);
      return false;
    }
  };

  const startOrQueueChat = async (requestMessages: ChatMessage[], sessionId: string, requestAttachments: AttachedFile[], skillIds: string[], clearDraft = true) => {
    if (!localModelId) return sendMessages(requestMessages, sessionId, requestAttachments, skillIds, clearDraft);
    try {
      const status = await invoke<{ loaded_model_id: string | null }>("local_model_status");
      if (status.loaded_model_id === localModelId) return sendMessages(requestMessages, sessionId, requestAttachments, skillIds, clearDraft);
      // Carried through the model-start pause rather than read at send time. The
      // skill was chosen for this message, and it is still that message when the
      // engine finally comes up -- reading a later state here would drop it.
      pendingChatRef.current = { messages: requestMessages, sessionId, attachments: requestAttachments, skillIds };
      setInput("");
      setStartModelPrompt(true);
      return true;
    } catch (reason) {
      onNotify("error", `Could not check local model status: ${String(reason)}`);
      return false;
    }
  };

  useEffect(() => {
    const unlistenPromise = listen<LocalModelEvent>("local-model-event", (event) => {
      const payload = event.payload;
      if (!pendingChatRef.current || payload.modelId !== localModelId) return;
      if (payload.kind === "loaded") {
        const pending = pendingChatRef.current;
        pendingChatRef.current = null;
        setStartModelPrompt(false);
        setStartingModel(false);
        setInput("");
        void sendMessages(pending.messages, pending.sessionId, pending.attachments, pending.skillIds);
      } else if (payload.kind === "failed") {
        setStartingModel(false);
        if (pendingChatRef.current) {
          const lastUserMessage = [...pendingChatRef.current.messages].reverse().find((message) => message.role === "user");
          if (lastUserMessage) setInput(lastUserMessage.content);
          addAttachments(pendingChatRef.current.attachments);
        }
        onNotify("error", payload.message || "The local model could not be started.");
      setStartModelPrompt(false);
      pendingChatRef.current = null;
      }
    });
    return () => { unlistenPromise.then((unlisten) => unlisten()); };
  }, [localModelId, onMessagesChange]);

  const confirmStartModel = async () => {
    if (!localModelId || startingModel) return;
    setStartModelPrompt(false);
    setStartingModel(true);
    try {
      await invoke("local_model_load", { id: localModelId });
    } catch (reason) {
      setStartingModel(false);
      if (pendingChatRef.current) {
        const pending = pendingChatRef.current;
        const lastUserMessage = [...pending.messages].reverse().find((message) => message.role === "user");
        if (lastUserMessage) setInput(lastUserMessage.content);
        addAttachments(pending.attachments);
        pendingChatRef.current = null;
      }
      onNotify("error", String(reason));
    }
  };

  const cancelStartModel = () => {
    setStartModelPrompt(false);
    setStartingModel(false);
    if (pendingChatRef.current) {
      const pending = pendingChatRef.current;
      const lastUserMessage = [...pending.messages].reverse().find((message) => message.role === "user");
      if (lastUserMessage) setInput(lastUserMessage.content);
      addAttachments(pending.attachments);
    }
    pendingChatRef.current = null;
  };

  /**
   * Starts a reply for `messageText` on top of `baseMessages`.
   *
   * Split out of `send` so a queued message can be dispatched from the run
   * listener, which holds the conversation it just finished rather than the
   * current component state. `targetSessionId` is the conversation to send to, or
   * `null` to create one -- the listener passes the finished run's session, which
   * may no longer be the one on screen.
   */
  const dispatch = async (
    messageText: string,
    requestAttachments: AttachedFile[],
    baseMessages: ChatMessage[],
    targetSessionId: string | null,
    clearDraft = true,
    hidden = false,
  ) => {
    const nextMessages = [...baseMessages];
    nextMessages.push({
      role: "user" as const,
      content: messageText,
      // A hidden turn is input the model needs but the user must not read as their
      // own words -- a finished sub-agent's report, whose result is shown on its
      // tool row instead.
      ...(hidden ? { hidden: true } : {}),
    });
    // The mode and access level travel with the creation, so a conversation is
    // never born in Chat and then corrected. With a conversation already open this
    // is a no-op -- `ensureSession` returns the existing id -- and the choice has
    // already been recorded against it by `changeMode`.
    const existing = targetSessionId;
    const sessionId = existing ?? onEnsureSession(nextMessages, { mode, permission });
    sessionIdRef.current = sessionId;
    // A conversation this send created is new to the remembered map, so the pair it
    // was created with is recorded against it here. Without this the restore effect
    // would find nothing for it and fall back to the stored header instead.
    if (!existing) rememberWorkingMode(mode, permission);
    onFirstSend();
    // Names the chat from the opening message, in parallel with the reply
    // rather than after it, so the sidebar updates sooner.
    titleSessionRef.current(sessionId, messageText);
    await startOrQueueChat(nextMessages, sessionId, requestAttachments, [], clearDraft);
  };
  dispatchRef.current = dispatch;

  const send = async () => {
    const text = input.trim();
    // Refused while the open conversation's transcript is still loading. Every
    // other check here is about *when* a reply may run; this one is about whether
    // the history is even known yet. Sending from an empty transcript would look
    // like a first message in a long conversation, and nothing on screen would
    // show that it was not.
    if (sessionLoadingRef.current) {
      onNotify("error", "Still loading this conversation. Try again in a moment.");
      return;
    }
    if ((!text && attachments.length === 0) || (!endpointId && !localModelId)) return;
    const requestAttachments = attachments;
    const messageText = text || `Please review the attached ${requestAttachments.length === 1 ? "file" : "files"}.`;
    const activeSession = sessionIdRef.current;
    // A reply is already running in this conversation. The message is queued
    // rather than refused, and `drainQueued` sends it the moment that reply ends
    // -- so a user can keep typing without waiting for the model to stop.
    if (activeSession && sessionRunIdsRef.current.has(activeSession)) {
      queuedMessagesRef.current.push({ sessionId: activeSession, text: messageText, attachments: requestAttachments });
      setQueuedCount(queuedMessagesRef.current.length);
      setInput("");
      removeSentAttachments(requestAttachments);
      return;
    }
    await dispatch(messageText, requestAttachments, messages, activeSession);
  };

  const skipReasoning = async () => {
    const sessionId = sessionIdRef.current;
    const runId = (sessionId && sessionRunIdsRef.current.get(sessionId)) || runIdRef.current;
    const run = backgroundRunsRef.current.get(runId);
    if (!run?.localModel || !run.isReasoning || !run.completionId) return;
    run.isReasoning = false;
    try {
      await invoke("ai_skip_local_reasoning", { runId });
    } catch (reason) {
      onNotify("error", String(reason));
    }
  };

  const stop = () => {
    const sessionId = sessionIdRef.current;
    const runId = (sessionId && sessionRunIdsRef.current.get(sessionId)) || runIdRef.current;
    if (!runId) return;
    const run = backgroundRunsRef.current.get(runId);
    backgroundRunsRef.current.delete(runId);
    // Closed here rather than waiting for the `stopped` event, because this
    // handler removes the run immediately and that event is then discarded by
    // the lookup at the top of the listener -- so nothing else would close it.
    // A step left running would keep counting from a timestamp nothing advances.
    if (run) run.activity = closeOpenSteps(run.activity);
    if (sessionId && sessionRunIdsRef.current.get(sessionId) === runId) sessionRunIdsRef.current.delete(sessionId);
    if (run && sessionId) onActivityRef.current(sessionId, null);
    setPendingApprovals([]);
    if (sessionId === sessionIdRef.current) {
      runIdRef.current = "";
      setRunning(false);
    }
    if (run && run.text) {
      // A stopped reply keeps whatever steps it took, for the same reason it
      // keeps its text: the transcript is the record of what happened, and a
      // partial answer that dropped its tool calls would misreport the run.
      const next = [...run.baseMessages, { role: "assistant" as const, content: run.text, reasoning: run.reasoning || undefined, ...(run.activity.length > 0 ? { activity: run.activity } : {}), modelId: run.modelLabel, providerId: run.providerLabel }];
      persistMessages(run.sessionId, next);
      if (run.sessionId === sessionIdRef.current) {
        setMessages(next);
        streamTextRef.current = "";
        streamReasoningRef.current = "";
        setStreamText("");
        setStreamReasoning("");
        setStreamActivity([]);
        setSearching(false);
      }
    } else if (run && run.sessionId === sessionIdRef.current) {
      streamTextRef.current = "";
      streamReasoningRef.current = "";
      setStreamText("");
      setStreamReasoning("");
      setStreamActivity([]);
      setSearching(false);
    }
    void invoke("ai_cancel_chat", { runId });
  };

  const retry = (assistantIndex: number) => {
    const sessionId = sessionIdRef.current;
    if (!sessionId || sessionRunIdsRef.current.has(sessionId)) return;
    const conversation = messages.slice(0, assistantIndex);
    if (!conversation.some((message) => message.role === "user")) return;
    sessionIdRef.current = sessionId;
    onFirstSend();
    void startOrQueueChat(conversation, sessionId, [], []);
  };

  const copyMessage = async (content: string, index: number) => {
    try {
      await navigator.clipboard.writeText(content);
      setCopiedIndex(index);
      window.setTimeout(() => setCopiedIndex((current) => current === index ? null : current), 1600);
    } catch {
      onNotify("error", "Could not copy this response.");
    }
  };

  // Selection inside a reply: only show the bar when the highlight is real,
  // and ignore selections the user made in the composer or their own message.
  const handleThreadSelection = (event: React.MouseEvent) => {
    const active = document.activeElement;
    if (active === inputRef.current || (active instanceof HTMLElement && active.closest("form"))) return;
    const node = event.target as HTMLElement;
    if (!node.closest(".markdown-content")) return;
    const picked = window.getSelection();
    const text = picked?.toString().trim() ?? "";
    if (!picked || picked.rangeCount === 0 || text.length === 0) {
      setSelection(null);
      return;
    }
    const rect = picked.getRangeAt(0).getBoundingClientRect();
    // A collapsed or offscreen rect means the highlight is stale.
    if (rect.width === 0 && rect.height === 0) {
      setSelection(null);
      return;
    }
    setSelection({ text, top: rect.top, left: rect.left + rect.width / 2 });
  };

  const clearSelection = () => {
    setSelection(null);
    window.getSelection()?.removeAllRanges();
  };

  // The bar follows a highlight, so it has to disappear when the highlight does --
  // which includes a plain click anywhere else on screen. A click outside the
  // reply does not run `handleThreadSelection` at all, so without this the only
  // way to dismiss it was clicking back inside the text, which is not what
  // dismissing a floating bar means to anyone.
  useEffect(() => {
    if (!selection) return;
    const dismiss = (event: MouseEvent) => {
      // The bar itself counts as inside, or pressing its own buttons would
      // dismiss it before the click landed on them.
      const target = event.target as Node;
      if (target instanceof Element && target.closest("[data-selection-bar]")) return;
      clearSelection();
    };
    // `pointerdown` rather than `click`: it fires before the browser collapses
    // the selection, so the bar is gone by the time the mouse is up. Waiting for
    // `click` left it visible for the gap between the two.
    document.addEventListener("pointerdown", dismiss);
    // Scrolling detaches the bar from the text it was anchored to, because its
    // position is a viewport coordinate captured at selection time. Leaving it
    // floating over unrelated text reads as a bug, so it goes with the scroll.
    threadRef.current?.addEventListener("scroll", clearSelection);
    return () => {
      document.removeEventListener("pointerdown", dismiss);
      threadRef.current?.removeEventListener("scroll", clearSelection);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selection]);

  // Builds a draft around the highlight and drops it in the composer for review.
  // Nothing is sent automatically, so the user can edit, send, or clear it.
  const draftFromSelection = (action: SelectionAction) => {
    if (!selection) return;
    const quoted = selection.text.length > 1200 ? `${selection.text.slice(0, 1200)}…` : selection.text;
    const lead = action === "explain" ? "Explain this in detail:" : "Summarize this in a few sentences:";
    setInput(`${lead}\n\n> ${quoted.replace(/\n/g, "\n> ")}\n\n`);
    setSelection(null);
    inputRef.current?.focus();
    // Put the caret after the draft rather than at the start.
    requestAnimationFrame(() => {
      const field = inputRef.current;
      if (!field) return;
      field.setSelectionRange(field.value.length, field.value.length);
    });
  };

  // The slash menu's state, derived from the draft so there is no second source
  // of truth: a bare `/word` that names no command yet opens the menu, and a
  // draft that *is* a command closes it and is drawn as a link instead.
  const slashTyped = slashQuery(input);
  const slashExact = exactCommand(input);
  const slashMatches =
    slashTyped !== null && slashExact === null ? matchCommands(slashTyped) : [];
  const slashActive = slashMatches.length > 0 ? Math.min(slashIndex, slashMatches.length - 1) : 0;

  /** Runs a command and clears the draft, because a command is not a message. */
  const runSlashCommand = (command: SlashCommand) => {
    const on = command.action === "web-search-on";
    setWebSearchEnabled(on);
    onNotify("success", on ? "Web search on" : "Web search off");
    setInput("");
    setSlashIndex(0);
    inputRef.current?.focus();
  };

  const handleSubmit = (event: React.FormEvent) => {
    event.preventDefault();
    // A draft that is exactly a command runs it rather than sending the text.
    if (slashExact) {
      runSlashCommand(slashExact);
      return;
    }
    void send();
  };

  // Grows the composer to fit the draft, capped by the field's max-height, and
  // collapses back to one row when it empties.
  const resizeComposer = () => {
    const field = inputRef.current;
    if (!field) return;
    field.style.height = "auto";
    field.style.height = `${field.scrollHeight}px`;
  };

  // Covers drafts that do not come from typing, such as one built from a
  // highlighted passage, and resets the height after a send clears the field.
  useEffect(resizeComposer, [input]);

  return (
    <main
      className="relative flex min-h-0 flex-1 flex-col"
      onDragEnter={(event) => { if (event.dataTransfer.types.includes("Files")) { event.preventDefault(); setDraggingFiles(true); } }}
      onDragOver={(event) => { if (event.dataTransfer.types.includes("Files")) { event.preventDefault(); event.dataTransfer.dropEffect = "copy"; setDraggingFiles(true); } }}
      onDragLeave={(event) => { if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setDraggingFiles(false); }}
      onDrop={(event) => {
        if (event.dataTransfer.types.includes("Files")) {
          event.preventDefault();
          void addBrowserFiles(Array.from(event.dataTransfer.files));
          setDraggingFiles(false);
        }
      }}
    >
      {draggingFiles && <div aria-hidden="true" className="pointer-events-none absolute inset-3 z-30 grid place-items-center rounded-xl border-2 border-dashed border-[var(--accent)] bg-[color-mix(in_srgb,var(--accent)_8%,var(--page))] text-sm font-medium text-[var(--text)]">Drop files to attach</div>}
      <div ref={threadRef} onScroll={handleThreadScroll} onMouseUp={handleThreadSelection} className="flex min-h-0 flex-1 flex-col overflow-y-auto overscroll-contain px-5 py-8 sm:px-8">
        {messages.length === 0 && !streamText ? (
          <section className="mx-auto flex w-full max-w-3xl flex-1 flex-col justify-center pb-16" aria-labelledby="welcome-title">
            {/* Two lines and nothing else. The empty screen is a moment of
                indecision, and it is resolved by the composer below, not by
                either a wall of suggested prompts or a description of the
                software. The headline asks the question; the line under it says
                only what the user needs to know to answer it. */}
            <h2 id="welcome-title" className="max-w-xl text-[28px] font-medium leading-[1.15] tracking-[-0.035em] sm:text-[34px]">What are we working on?</h2>
            <p className="mt-3 max-w-md text-[14px] leading-6 text-[var(--muted)]">
              {endpointId || localModelId
                ? "Ask a question, or switch to Agent to let the model read and edit your workspace."
                : "Choose a model below to begin. Your conversations stay on this device."}
            </p>
            {!endpointId && !localModelId && (
              <button type="button" onClick={onOpenSettings} className="mt-6 inline-flex min-h-9 w-fit items-center rounded-lg border border-[var(--line)] px-3.5 text-[13px] font-medium text-[var(--text)] transition-colors hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]">Set up a provider</button>
            )}
          </section>
        ) : (
          <div className="mx-auto flex w-full max-w-3xl flex-col gap-7 py-3">
            {/* User and assistant only. A web search used to be stored as its own `tool`
                      message, which drew a card floating above the reply and
                      outside the activity panel that holds every other tool;
                      searches are now step data inside that panel. A conversation
                      saved before that change still holds one, and it is filtered
                      out rather than drawn — an old card in a new layout is worse
                      than no card, and the search is still recorded in the reply's
                      own activity steps. */}
            {messages.filter((message) => message.role === "user" || message.role === "assistant").map((message, index) => (
              message.hidden ? (
                // A background sub-agent's report. Drawn as a centred notice, not a
                // message from either side: the report itself lives on the `sub_agent`
                // tool row inside the reply that spawned it, and this only marks that
                // the run has reported back and the model has been told.
                <div key={`${index}-notice`} className="my-2 flex items-center gap-3 px-1 text-[11px] text-[var(--quiet)]">
                  <span className="h-px flex-1 bg-[var(--line)]" />
                  <span className="shrink-0">{message.content.split("\n")[0]}</span>
                  <span className="h-px flex-1 bg-[var(--line)]" />
                </div>
              ) : (
              <article key={`${index}-${message.role}`} className={`flex ${message.role === "user" ? "justify-end" : "justify-start"}`}>
                <div className={`group max-w-[85%] break-words text-[14px] leading-7 sm:max-w-[78%] ${message.role === "user" ? "whitespace-pre-wrap rounded-2xl rounded-br-md bg-[var(--raised)] px-4 py-3 text-[var(--text)]" : "py-1 text-[var(--text)]"}`}>
                  {message.role === "assistant" ? <>
                    {/* The panel supersedes the old inline reasoning block. A thought
                        that ran before a tool call and one that ran after it are
                        separate steps, which a single pre-rendered `reasoning`
                        string cannot express — it has no way to say where the
                        thinking was interrupted. */}
                    <ActivityPanel steps={message.activity ?? []} live={false} renderMarkdown={renderMarkdown} totalSeconds={message.metrics?.elapsed_seconds} />
                    <div className="markdown-content">{renderMarkdown(message.content)}</div>
                    {/* After the text, not above it and not inside the panel. The
                        panel is what the agent did, in order; this is what it added
                        up to, which is a fact about the reply rather than a step
                        in it. Above the answer it competed with the answer, and
                        inside the panel you had to open it to learn anything was
                        written at all. */}
                    <FileChanges steps={message.activity ?? []} live={false} onOpenFile={onOpenFile} />
                    {/* Metadata and actions stay hidden until the message is hovered. `group-focus-within` keeps
                        them reachable for keyboard users, and the `min-h-8` row reserves the space up front so
                        revealing the bar never reflows the message text. */}
                    <div className="mt-2 flex min-h-8 flex-wrap items-center gap-x-3 gap-y-1 text-[11px] text-[var(--quiet)] opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100">
                      {showMetrics && message.metrics && <span title={message.metrics.generation_rate_estimated ? "Generation rate estimated from response length" : "Generation rate reported by the local model server"}>{message.metrics.prompt_tokens != null ? `${message.metrics.prompt_tokens.toLocaleString()} prompt · ` : ""}{message.metrics.completion_tokens != null ? `${message.metrics.completion_tokens.toLocaleString()} completion · ` : ""}Prompt {message.metrics.prompt_seconds.toFixed(1)}s · Generation {message.metrics.generation_seconds.toFixed(1)}s · {message.metrics.generation_rate_estimated ? "~" : ""}{message.metrics.tokens_per_second.toFixed(1)} tok/s{message.metrics.prompt_eval_tokens_per_second != null ? ` · PP ${message.metrics.prompt_eval_tokens_per_second.toFixed(1)} tok/s` : ""}</span>}
                      <div className="flex items-center gap-1">
                        <button type="button" onClick={() => void copyMessage(message.content, index)} className="inline-flex min-h-8 items-center gap-1 rounded px-1.5 hover:bg-[var(--raised)] hover:text-[var(--text)]" aria-label={copiedIndex === index ? "Copied response" : "Copy response"} title={copiedIndex === index ? "Copied" : "Copy response"}>{copiedIndex === index ? <Check size={13} /> : <Clipboard size={13} />}<span>{copiedIndex === index ? "Copied" : "Copy"}</span></button>
                        <button type="button" onClick={() => retry(index)} disabled={running} className="inline-flex min-h-8 items-center gap-1 rounded px-1.5 hover:bg-[var(--raised)] hover:text-[var(--text)] disabled:opacity-40" aria-label="Retry response" title="Retry response"><RotateCcw size={13} /><span>Retry</span></button>
                      </div>
                    </div>
                  {/* One branch of one chain, so a user message cannot be drawn
                      twice: an earlier version rendered `UserText` here *and*
                      fell through to this branch, which repeated the question.
                      The user's own text goes through `userTextOnly` because a
                      stored message carries the skills applied to it, which the
                      model needs and the reader does not. */}
                  </> : userTextOnly(message.content)}
                </div>
              </article>
              )
            ))}
            {(streamText || streamActivity.length > 0) && <article className="flex justify-start"><div className="max-w-[78%] break-words py-1 text-[14px] leading-7">
              <ActivityPanel steps={streamActivity} live={running} renderMarkdown={renderMarkdown} />
              {streamText && <div className="markdown-content">{renderMarkdown(streamText)}</div>}
              {/* Live as well as stored: the files a run has already written are
                  the interesting part of a run in progress, and waiting for it to
                  finish to find out means scrolling back up the transcript. The
                  counts move as more writes land. */}
              <FileChanges steps={streamActivity} live={running} onOpenFile={onOpenFile} />
            </div></article>}
            {running && !streamText && streamActivity.length === 0 && <div role="status" className="flex items-center gap-3 text-sm text-[var(--muted)]"><LoaderCircle size={16} className="animate-spin text-[var(--accent)]" />{searching ? "Searching the web" : "Thinking"}</div>}
          </div>
        )}
      </div>

      {selection && <SelectionActions
        text={selection.text}
        top={selection.top}
        left={selection.left}
        onCopy={() => { void navigator.clipboard.writeText(selection.text).then(clearSelection, () => onNotify("error", "Could not copy the selection.")); }}
        onAction={draftFromSelection}
        onDismiss={clearSelection}
      />}

      <div className="px-5 pb-2 sm:px-8 sm:pb-2">
        <style>{MARKDOWN_STYLES}</style>
        <form onSubmit={handleSubmit} className="mx-auto w-full max-w-3xl">
          {/* The prompt sits above the composer rather than inside it. Inside, it
              read as another attachment on a draft the user is still editing; above,
              it is plainly a question about the run, and the composer stays
              exactly as it was. The gap is one `mb` because it belongs to the
              transcript above, not to the control below. */}
          {pendingApprovals.length > 0 && (
            <div className="mb-2">
              <ToolApproval
                request={pendingApprovals[0]}
                onAnswer={(approvalId, allow) => {
                  void invoke("ai_answer_approval", { approvalId, allow }).catch(() => {
                    // The run was already stopped, so there was nothing to answer.
                    // Closing the prompt is the right outcome either way.
                  });
                  dismissApproval(approvalId);
                }}
                onCancel={() => dismissApproval(pendingApprovals[0].approvalId)}
              />
            </div>
          )}
          {/* The composer is a two-row card: the draft on top, controls on the
              bottom line. Radius is deliberately tighter than the message bubbles
              so the input reads as a control surface rather than a chat bubble. */}
          <div className="relative rounded-lg border border-[var(--line)] bg-[var(--panel)] px-2 py-1.5 shadow-[0_8px_24px_-14px_rgba(0,0,0,0.5)] transition-colors focus-within:border-[color-mix(in_srgb,var(--accent)_30%,var(--line))]">
            {slashMatches.length > 0 && (
              <SlashMenu commands={slashMatches} activeIndex={slashActive} onChoose={runSlashCommand} />
            )}
            {queuedCount > 0 && (
              <div className="px-2.5 pt-2 text-[11px] text-[var(--quiet)]">
                {queuedCount} {queuedCount === 1 ? "message" : "messages"} queued — sent when this reply ends
              </div>
            )}
            {attachments.length > 0 && <div className="flex flex-wrap gap-2 px-2 pt-2">
              {attachments.map((file) => <span key={file.path} title={file.path} className="inline-flex max-w-56 items-center gap-1.5 rounded-md border border-[var(--line)] bg-[var(--rail)] py-1 pl-2 pr-1.5 text-xs text-[var(--muted)]"><Paperclip size={12} className="shrink-0" /><span className="truncate">{file.name}</span><button type="button" onClick={() => removeAttachment(file.path)} aria-label={`Remove ${file.name}`} className="grid size-6 shrink-0 place-items-center rounded text-[var(--quiet)] hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]"><X size={13} /></button></span>)}
            </div>}
            <div className="flex items-start gap-1">
            {/* Attachments are the only composer control now: the `+` menu and its
                skills / web-search rows are gone, so this opens the file dialog
                directly. Web search is on for the run itself. */}
            <button type="button" onClick={() => void chooseFiles()} aria-label="Attach files" title="Attach files" className="mt-1.5 grid size-8 shrink-0 place-items-center rounded-md text-[var(--muted)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"><Paperclip size={18} /></button>
            <textarea
              ref={inputRef}
              aria-label="Message"
              rows={1}
              value={input}
              onChange={(event) => setInput(event.target.value)}
              onInput={resizeComposer}
              onKeyDown={(event) => {
                // While the menu is open it owns Enter and the arrow keys; a
                // finished command owns Enter too, running instead of sending.
                if (slashMatches.length > 0) {
                  if (event.key === "ArrowDown" || event.key === "ArrowUp") {
                    event.preventDefault();
                    const step = event.key === "ArrowDown" ? 1 : -1;
                    setSlashIndex((current) => {
                      const next = current + step;
                      if (next < 0) return slashMatches.length - 1;
                      if (next >= slashMatches.length) return 0;
                      return next;
                    });
                    return;
                  }
                  if (event.key === "Escape") {
                    event.preventDefault();
                    setInput("");
                    return;
                  }
                  if (event.key === "Enter" && !event.shiftKey) {
                    event.preventDefault();
                    runSlashCommand(slashMatches[slashActive]);
                    return;
                  }
                }
                if (event.key !== "Enter") return;
                if (slashExact) {
                  event.preventDefault();
                  runSlashCommand(slashExact);
                  return;
                }
                // Whichever key is not bound to sending keeps its normal meaning, so
                // Ctrl + Enter still inserts a newline when plain Enter sends.
                const sends = sendKey === "Enter" ? !event.shiftKey : event.ctrlKey || event.metaKey;
                if (!sends) return;
                event.preventDefault();
                void send();
              }}
              placeholder={sessionLoading ? "Loading conversation…" : endpointId || localModelId ? "Write a message…" : "Choose a model to begin"}
              disabled={sessionLoading || (!endpointId && !localModelId)}
              // Grows with the draft up to five rows, then scrolls. A draft that
              // is exactly a command is drawn as a link -- the accent and the
              // underline are what say "this is a control, not a message".
              className={`max-h-[9.5rem] min-h-11 min-w-0 flex-1 resize-none overflow-y-auto bg-transparent px-1.5 py-2.5 text-[15px] leading-6 outline-none placeholder:text-[var(--quiet)] disabled:cursor-not-allowed ${
                slashExact ? "text-[var(--accent)] underline underline-offset-4" : "text-[var(--text)]"
              }`}
            />
            {running && localModelId && backgroundRunsRef.current.get(runIdRef.current)?.isReasoning && <button type="button" onClick={() => void skipReasoning()} className="mt-0.5 inline-flex h-9 shrink-0 items-center gap-1.5 rounded-full bg-[var(--raised)] px-3 text-xs text-[var(--text)] transition-colors hover:bg-[color-mix(in_srgb,var(--accent)_20%,var(--raised))] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]" aria-label="Skip reasoning" title="End reasoning and continue the answer"><SkipForward size={14} />Skip reasoning</button>}
            {running && <button type="button" onClick={stop} className="mt-0.5 grid size-9 shrink-0 place-items-center rounded-full bg-[var(--raised)] text-[var(--text)] transition-colors hover:bg-[color-mix(in_srgb,var(--accent)_20%,var(--raised))] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]" aria-label="Stop generating" title="Stop generating"><Square size={14} fill="currentColor" /></button>}
            {/* While a reply runs the send button queues rather than disappearing, so
                a message typed mid-reply is neither lost nor blocked on the model
                stopping. A plain click or Enter adds it to the queue. */}
            {!slashExact && (running ? input.trim().length > 0 : input.trim().length > 0 || attachments.length > 0) && <button type="submit" disabled={sessionLoading || (!endpointId && !localModelId)} className="mt-0.5 grid size-9 shrink-0 place-items-center rounded-full bg-[var(--text)] text-[var(--page)] transition-colors hover:bg-[var(--accent)] hover:text-[var(--accent-ink)] disabled:cursor-not-allowed disabled:opacity-35 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]" aria-label={running ? "Queue message" : "Send message"} title={running ? "Queue message" : "Send message"}><ArrowUp size={18} strokeWidth={2.5} /></button>}
            </div>
          </div>
          <div className="flex min-h-8 items-center justify-between gap-2 px-1">
            <ModeSelector
              mode={mode}
              permission={permission}
              canUseTools={canUseTools}
              onModeChange={changeMode}
              onPermissionChange={changePermission}
            />
            <div className="flex min-h-8 items-center gap-2">
            <ModelSwitcher selectedEndpoint={endpointId} selectedModel={model} localModelId={localModelId} refreshKey={modelRefreshKey} onSelect={onModelSelect} onSelectLocal={(selected) => { onModelSelect("", ""); onLocalModelSelect(selected.id); }} />
            {canSetEffort && <EffortControl values={effortValues} value={effort} onChange={setEffort} />}
            <ContextRing tokens={contextTokens} limit={displayedContextLimit} estimated={contextIsEstimated} isLocal={Boolean(localModelId)} />
            </div>
          </div>
        </form>
      </div>
      {startModelPrompt && <StartModelDialog starting={startingModel} onConfirm={() => void confirmStartModel()} onCancel={cancelStartModel} />}
    </main>
  );
}
