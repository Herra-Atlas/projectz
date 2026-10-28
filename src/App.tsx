import { Suspense, lazy, useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { ChevronDown, Power } from "lucide-react";
import NotificationStack from "./features/notifications/NotificationStack";
import ErrorBoundary from "./components/ErrorBoundary";
import { NotificationHostProvider } from "./features/notifications/notificationHost";
import { useCopiedNotification, useNotifications } from "./features/notifications/useNotifications";
import Sidebar from "./components/Sidebar";
import TooltipLayer from "./components/TooltipLayer";
import FindBar from "./components/find/FindBar";
import ChatPage from "./pages/chat/ChatPage";
import SettingsModal from "./components/SettingsModal";
import StartupScreen from "./components/StartupScreen";
import DatabasePage from "./pages/DatabasePage";
import JobsPage from "./pages/jobs/JobsPage";
import WorkspacePicker from "./components/WorkspacePicker";
import RightPanel from "./components/rightPanel/RightPanel";
import { useChatSessions } from "./features/chat/useChatSessions";
import { chatRepository } from "./features/chat/chatRepository";
import { usePreferences } from "./features/models/usePreferences";
import { useAppearance } from "./features/appearance/useAppearance";
import { useRightPanel } from "./features/rightPanel/useRightPanel";
import { displayModelName } from "./features/models/modelName";
import { useWorkspaces, workspaceLabel } from "./features/workspace/useWorkspaces";
import { loadGeneralSettings } from "./features/settings/generalSettings";
import { loadNotificationSettings } from "./features/notifications/settings";
import { createPortal } from "react-dom";
import type { ChatMessage, SessionActivity, SessionRunStatus } from "./features/chat/types";
import type { WorkingMode } from "./features/chat/chatMode";
import "./App.css";

/** The statistics page pulls in Recharts, which is by far the heaviest
    dependency in the app and is only needed when that view is open. Loading it
    lazily keeps it out of the chat bundle entirely rather than weighing down
    the view people spend all their time in. */
const StatisticsPage = lazy(() => import("./pages/statistics/StatisticsPage"));

/** Placeholder while the statistics chunk and its first query are in flight. */
function PageFallback() {
  return (
    <div className="flex min-h-0 flex-1 items-center justify-center text-sm text-[var(--quiet)]">Loading…</div>
  );
}

type LocalRuntimeStatus = { loaded_model_id: string | null; loading_model_id: string | null; selected_model_id: string | null; base_url: string; started_at: number | null };
type LocalModelEvent = { kind: "loading" | "loaded" | "failed" | "unloaded"; modelId?: string; modelPath?: string; message?: string };
type StartupProgress = { phase: string; message: string; progress: number };

/**
 * "Running for 4m 12s", ticking once a second while it shows.
 *
 * A fresh stamp arrives through the poll once a server has been started by
 * this app; an orphaned server from a previous session has no stamp, and its
 * row deliberately carries no duration rather than one it invented.
 */
function ResidentFor({ startedAt }: { startedAt: number }) {
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000));
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Math.floor(Date.now() / 1000)), 1000);
    return () => window.clearInterval(timer);
  }, []);
  const seconds = Math.max(0, now - startedAt);
  const parts: string[] = [];
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  if (hours > 0) parts.push(`${hours}h`);
  if (minutes > 0 || hours > 0) parts.push(`${minutes}m`);
  parts.push(`${seconds % 60}s`);
  return <span> · for {parts.join(" ")}</span>;
}

/**
 * The host provider sits above the shell so a dialog opened anywhere in the tree
 * can claim the notification stack's portal target -- see
 * `features/notifications/notificationHost.tsx` for why a `showModal()` dialog
 * needs this at all.
 */
export default function App() {
  return (
    <ErrorBoundary>
      <NotificationHostProvider>
        <AppShell />
      </NotificationHostProvider>
    </ErrorBoundary>
  );
}

function AppShell() {
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [startupDone, setStartupDone] = useState(false);
  const [startupError, setStartupError] = useState("");
  const [sessionReloadKey, setSessionReloadKey] = useState(0);
  const [startupProgress, setStartupProgress] = useState<StartupProgress>({ phase: "database", message: "Opening database", progress: 4 });
  const [view, setView] = useState<"chat" | "database" | "statistics" | "jobs">("chat");
  const [endpointId, setEndpointId] = useState("");
  const [model, setModel] = useState("");
  const [localModelId, setLocalModelId] = useState("");
  const [endpointRefreshKey, setEndpointRefreshKey] = useState(0);
  const [localRuntime, setLocalRuntime] = useState<LocalRuntimeStatus>({ loaded_model_id: null, loading_model_id: null, selected_model_id: null, base_url: "http://127.0.0.1:43127/v1", started_at: null });
  /**
   * A request to show the model's terminal, raised on every load attempt.
   *
   * A request rather than relying on `loading_model_id` alone, because that
   * value can be back to null before the panel ever renders -- a load that
   * fails immediately emits `loading` and `failed` in one tick, and React
   * batches the two state changes. On a retry the panel then never opened,
   * which looked like the terminal being broken rather than the attempt being
   * short. The key is what makes a second attempt a new request.
   */
  const [terminalRequest, setTerminalRequest] = useState<{ id: string; key: number } | null>(null);
  // The corner stack lives in its own module rather than here, so a view like the
  // settings modal can be handed a `notify` without the stack's shape being a
  // detail of the app shell. `App` renders it once and owns the wiring.
  const { notifications, notify, notifyAction, dismiss: dismissNotification, clearKey: clearNotification } = useNotifications();
  const { copiedId: copiedNotificationId, markCopied } = useCopiedNotification();
  const [sessionActivity, setSessionActivity] = useState<SessionActivity>({});
  // The conversation the right panel's Overview view is showing, plus a nonce so
  // asking for the same one again still brings the view forward. Held here rather
  // than in the panel because it is raised from two places -- the sidebar and the
  // statistics page -- and both should reach one view through one path.
  const [overviewRequest, setOverviewRequest] = useState<{ id: string; key: number } | null>(null);
  const chat = useChatSessions(startupDone, sessionReloadKey, localModelId || model, endpointId);
  // Preferences live here so the chat view and the settings modal always agree
  // on the title-model choice without one re-reading a stale copy.
  const { preferences, savePreferences } = usePreferences(startupDone);
  // Theme and accent, applied to the document element rather than passed to the
  // tree: every screen already reads the tokens they set, so this is what makes
  // the change cover surfaces this component has never heard of.
  const { appearance, saveAppearance } = useAppearance(startupDone);
  const rightPanel = useRightPanel();
  // The file the transcript asked the panel to show, and the acknowledgement that
  // clears it. Held here rather than inside `RightPanel` because the request comes
  // from the chat column beside it, and the two must not hold references to each
  // other to pass one string.
  const [requestedFile, setRequestedFile] = useState<string | null>(null);
  const [requestedUrl, setRequestedUrl] = useState<{ url: string; key: number } | null>(null);
  /**
   * The skill whose folder the Files panel should draw, or null for the workspace.
   *
   * Set by the Skills page when a skill is opened for editing. Held here for the
   * same reason `requestedFile` is: the click happens inside the settings dialog
   * and the tree lives beside the chat column, so neither can reach the other
   * without a value in the component that owns both.
   */
  const [openSkill, setOpenSkill] = useState<string | null>(null);
  // The folder the agent may read and write. Loaded once the app has started,
  // because the backend reads the stored selection during `setup` and the header
  // must not render a folder the tools are not actually using.
  //
  // `root` is deliberately not read here. It is the live root rather than the
  // selection, and with no default it is empty exactly when nothing is chosen --
  // so rendering it would put the header and the picker describing two different
  // states. The selection is the only thing the user chose, so it is the only
  // thing shown.
  const { workspaces, error: workspaceError, open: openWorkspace, select: selectWorkspace, close: closeWorkspace } = useWorkspaces(startupDone);
  const [workspaceMenuOpen, setWorkspaceMenuOpen] = useState(false);
  const workspaceMenuRef = useRef<HTMLDivElement>(null);

  /**
   * Shows the folder chooser and opens whatever comes back.
   *
   * One function for both entry points -- the sidebar button and the picker's own
   * "Open folder…" row -- because two copies of a dialog call would drift, and
   * the picker is rendered inside the header while the button is in the sidebar.
   *
   * A cancelled dialog returns `null` and is not an error: putting "Could not
   * open folder" on screen because someone pressed Cancel would be wrong.
   */
  const chooseFolder = useCallback(async () => {
    const chosen = await openDialog({ directory: true, multiple: false });
    // `null` is the user pressing Cancel, which is not an error and must not put
    // a message on screen. A failure inside `open` is already recorded and shown
    // by the header, so nothing is thrown here either.
    if (typeof chosen === "string") await openWorkspace(chosen);
  }, [openWorkspace]);
  const activeIdRef = useRef(chat.activeId);
  activeIdRef.current = chat.activeId;

  const handleSessionActivity = useCallback((sessionId: string, status: SessionRunStatus | null) => {
    setSessionActivity((current) => {
      if (status === null) {
        if (!(sessionId in current)) return current;
        const next = { ...current };
        delete next[sessionId];
        return next;
      }
      if (status === "done" || status === "error") {
        if (current[sessionId] === status) return current;
        // The active conversation draws its own state rather than a dot, so a
        // terminal status is not stored for it. But a *live* mark must still be
        // cleared: a run that ended while this view was not listening (a stop
        // during an unmount, a reply that failed in the background) left
        // "streaming" behind, and ignoring it here is what kept the sidebar
        // spinner going after Cancel.
        if (sessionId === activeIdRef.current) {
          if (current[sessionId] !== "streaming" && current[sessionId] !== "searching") return current;
          const next = { ...current };
          delete next[sessionId];
          return next;
        }
        return { ...current, [sessionId]: status };
      }
      if (current[sessionId] === status) return current;
      return { ...current, [sessionId]: status };
    });
  }, []);

  const handleSelectSession = useCallback((id: string | null) => {
    chat.selectSession(id);
    if (!id) return;
    setSessionActivity((current) => {
      const status = current[id];
      if (status !== "done" && status !== "error") return current;
      const next = { ...current };
      delete next[id];
      return next;
    });
  }, [chat.selectSession]);

  /**
   * Opens the right panel on a conversation's read-only overview.
   *
   * Only a request is raised. The panel owns the view and reads the transcript
   * through the same `loadSessionForOverview` the chat view uses, so the sidebar
   * and the statistics page cannot drift into different figures for the same
   * conversation. The nonce is what makes a second click re-open a conversation
   * that is already being shown.
   */
  const handleOpenOverview = useCallback((id: string) => {
    setOverviewRequest({ id, key: Date.now() });
  }, []);

  /**
   * Copies a notification's text and flags the row for a moment.
   *
   * The row itself owns the confirmation, but the timer is here in the hook
   * because a `setTimeout` that outlives the click that started it has no
   * business living inside the component that draws the text.
   */
  const copyNotification = useCallback((notification: { id: number; message: string }) => {
    void navigator.clipboard.writeText(notification.message)
      .then(() => markCopied(notification.id))
      .catch(() => notify("error", "Could not copy notification."));
  }, [markCopied, notify]);

  useEffect(() => {
    let mounted = true;
    let unlistenProgress: (() => void) | null = null;
    let unlistenLocal: (() => void) | null = null;
    let unlistenJob: (() => void) | null = null;
    let unlistenMessaging: (() => void) | null = null;
    let unlistenAi: (() => void) | null = null;
    void Promise.all([
      listen<StartupProgress>("startup-progress", (event) => setStartupProgress(event.payload)).then((unlisten) => {
        if (mounted) unlistenProgress = unlisten;
        else unlisten();
      }),
      listen<LocalModelEvent>("local-model-event", (event) => {
        const payload = event.payload;
        if (payload.kind === "loading") {
          setLocalRuntime((current) => ({ ...current, loading_model_id: payload.modelId ?? null }));
          // Every attempt asks for the terminal, even one that will fail at
          // once: the output of a failed start is exactly what the user opens
          // it to read.
          if (payload.modelId) setTerminalRequest({ id: payload.modelId, key: Date.now() });
        } else if (payload.kind === "loaded") {
          setLocalRuntime((current) => ({ ...current, loaded_model_id: payload.modelId ?? null, loading_model_id: null }));
          notify("success", "Local model ready");
        } else if (payload.kind === "unloaded") {
          setLocalRuntime((current) => ({ ...current, loaded_model_id: null, loading_model_id: null, started_at: null }));
          setResidentServers((current) => ({ ...current, llama: null, llamaStartedAt: null }));
        } else {
          setLocalRuntime((current) => ({ ...current, loaded_model_id: null, loading_model_id: null, started_at: null }));
          setResidentServers((current) => ({ ...current, llama: null, llamaStartedAt: null }));
          notify("error", payload.message || "Local model failed to load");
        }
      }).then((unlisten) => {
        if (mounted) unlistenLocal = unlisten;
        else unlisten();
      }),
      // A scheduled run writes its own transcript with no window watching, so this
      // is the only word the sidebar gets that a conversation it has never heard of
      // now exists. Without it a finished job appears to have produced nothing until
      // something else happens to reload the list.
      listen("job-event", () => {
        if (mounted) setSessionReloadKey((key) => key + 1);
      }).then((unlisten) => {
        if (mounted) unlistenJob = unlisten;
        else unlisten();
      }),
      // A message sent from outside the app writes a conversation with no window
      // watching, which is the same situation as a job's. `"state"` is only the
      // settings record moving, which is not a new conversation and must not
      // refetch the list.
      listen<{ kind?: string }>("messaging-event", (event) => {
        if (mounted && event.payload?.kind !== "state") setSessionReloadKey((key) => key + 1);
      }).then((unlisten) => {
        if (mounted) unlistenMessaging = unlisten;
        else unlisten();
      }),
      // **Any run, whichever door it came through.** The chat view reports its own
      // activity, but a job or a message from a phone has no view to report from --
      // its spinner on the sidebar used to appear only after a restart, when the
      // transcript was finally read. The event carries the session, so this is the
      // one place that knows a conversation is busy regardless of who started it.
      listen<{ session_id?: string | null; kind: string }>("ai-event", (event) => {
        if (!mounted) return;
        const sessionId = event.payload.session_id;
        if (!sessionId) return;
        if (event.payload.kind === "failed") handleSessionActivity(sessionId, "error");
        else if (event.payload.kind === "completed" || event.payload.kind === "stopped") handleSessionActivity(sessionId, "done");
        else if (
          event.payload.kind === "status" ||
          event.payload.kind === "delta" ||
          event.payload.kind === "tool_call" ||
          event.payload.kind === "tool_result" ||
          event.payload.kind === "reasoning_delta" ||
          event.payload.kind === "retrying"
        ) {
          handleSessionActivity(sessionId, "streaming");
        }
      }).then((unlisten) => {
        if (mounted) unlistenAi = unlisten;
        else unlisten();
      }),
    ]).then(() => mounted ? invoke("database_initialize") : Promise.reject(new Error("Startup cancelled"))).then(() => {
      setStartupProgress({ phase: "migration", message: "Importing existing data", progress: 94 });
      return chatRepository.initialize();
    }).then(() => {
      // Behind the startup screen, so the chat page's greeting and the composer's
      // preferences are already in memory when the app first renders. Read after
      // the import above, which is what migrates this record on an old install.
      //
      // The notification record is primed here too, because a reply can finish
      // before any settings page has been opened and its completion sound is read
      // from the cache at that moment.
      return Promise.all([
        loadGeneralSettings(),
        loadNotificationSettings(),
      ]).catch(() => undefined);
    }).then(() => {
      if (!mounted) return;
      setStartupProgress({ phase: "ready", message: "Opening ProjectZ", progress: 100 });
      setStartupDone(true);
    }).catch((reason: unknown) => {
      if (mounted) setStartupError(String(reason));
    });
    return () => {
      mounted = false;
      unlistenProgress?.();
      unlistenLocal?.();
      unlistenJob?.();
      unlistenMessaging?.();
      unlistenAi?.();
    };
  }, [notify, handleSessionActivity]);

  useEffect(() => {
    if (!startupDone) return;
    Promise.all([
      invoke<LocalRuntimeStatus>("local_model_status"),
      invoke<{ id: string; models: string[] }[]>("ai_list_endpoints"),
      invoke<{ id: string }[]>("local_models_list"),
    ]).then(([status, providers, localModels]) => {
      setLocalRuntime(status);
      try {
        const selected = JSON.parse(localStorage.getItem("projectz.selected-model.v1") ?? "null");
        if (selected?.localModelId && localModels.some((item) => item.id === selected.localModelId)) {
          setLocalModelId(selected.localModelId);
          return;
        }
        // The provider's `models` list already excludes anything switched off, so
        // membership is the whole test.
        if (selected?.endpointId && selected?.model && providers.some((provider) => provider.id === selected.endpointId && provider.models.includes(selected.model))) {
          setEndpointId(selected.endpointId);
          setModel(selected.model);
          return;
        }
        localStorage.removeItem("projectz.selected-model.v1");
      } catch { }
        const fallbackLocalId = status.selected_model_id ?? status.loaded_model_id ?? "";
        setLocalModelId(localModels.some((item) => item.id === fallbackLocalId) ? fallbackLocalId : "");
      }).catch(() => undefined);
  }, [startupDone]);

  // The Model section's main choice is what new conversations open on.
  //
  // A remembered selection still wins on launch -- the model the previous
  // session ended on is the honest "where you were" -- so this applies while
  // the composer has no conversation open, whatever launch remembered. A
  // mid-conversation switch stays the switcher's decision; the preference is
  // not a lock on an in-progress chat.
  useEffect(() => {
    if (!startupDone || chat.activeId !== null) return;
    const preferred = preferences.mainModel;
    if (!preferred) return;
    if (preferred.localModelId) {
      // A local selection is honoured only where the model still exists; a
      // deleted one has no fallback of its own, so it is dropped rather than
      // guessing a model nobody named.
      void invoke<{ id: string }[]>("local_models_list")
        .then((models) => {
          if (models.some((item) => item.id === preferred.localModelId)) {
            setLocalModelId(preferred.localModelId!);
          }
        })
        .catch(() => undefined);
      return;
    }
    void invoke<{ id: string; models: string[] }[]>("ai_list_endpoints")
      .then((providers) => {
        const provider = providers.find((item) => item.id === preferred.endpointId);
        // Membership is the whole test: the provider's list already excludes
        // anything switched off, exactly as the launch read above does.
        if (provider && preferred.model && provider.models.includes(preferred.model)) {
          setEndpointId(preferred.endpointId!);
          setModel(preferred.model);
          setLocalModelId("");
        }
      })
      .catch(() => undefined);
  }, [startupDone, chat.activeId, preferences.mainModel]);

  const ensureSession = useCallback((messages: ChatMessage[], workingMode: WorkingMode) => {
    if (chat.activeId && chat.activeSession) return chat.activeSession.id;
    return chat.createSessionWithMessages(messages, workingMode);
  }, [chat.activeId, chat.activeSession, chat.createSessionWithMessages]);

  const handleMessagesChange = useCallback((sessionId: string, messages: ChatMessage[]) => {
    chat.updateMessages(sessionId, messages);
  }, [chat.updateMessages]);

  const unloadLocalModel = async () => {
    try {
      await invoke("local_model_unload");
      setLocalRuntime((current) => ({ ...current, loaded_model_id: null, loading_model_id: null, started_at: null }));
      setResidentServers((current) => ({ ...current, llama: null, llamaStartedAt: null }));
    } catch (reason) {
      notify("error", String(reason));
    }
  };

  /**
   * What is resident right now, in the header: the warm llama.cpp model and
   * each running audio.cpp process. A slow poll of cheap status reads -- this
   * corner is a glance, not a dashboard -- with the llama name resolved through
   * the same list the chat page keeps.
   *
   * The two audio purposes are two processes with two resident models, so they
   * are held apart here. One combined "canary asr + sherpa kws" could not be
   * stopped a half at a time, and the row a reader saw was not the process
   * they could end.
   */
  const [residentServers, setResidentServers] = useState<{
    llama: string | null;
    llamaStartedAt: number | null;
    speech: { model: string | null; startedAt: number | null } | null;
    wake: { model: string | null; startedAt: number | null } | null;
  }>({ llama: null, llamaStartedAt: null, speech: null, wake: null });
  const [serversMenuOpen, setServersMenuOpen] = useState(false);
  /** Where the portaled menu hangs: measured from the chip on every open, so
      window resizes or a title wrapping to two lines never strand it. */
  const serversChipRef = useRef<HTMLButtonElement | null>(null);
  const [serversMenu, setServersMenu] = useState<{ top: number; right: number }>({ top: 60, right: 20 });

  useEffect(() => {
    let live = true;
    const poll = async () => {
      // Each side answers independently; a failure on one must not blank the
      // other, so the last known figures stay.
      const [llama, speech, wake] = await Promise.allSettled([
        invoke<{ loaded_model_id: string | null; loading_model_id: string | null; started_at: number | null }>("local_model_status"),
        invoke<{ running: boolean; startedAt: number | null; loaded: string | null }>("audio_status", { purpose: "speech" }),
        invoke<{ running: boolean; startedAt: number | null; loaded: string | null }>("audio_status", { purpose: "wake" }),
      ]);
      if (!live) return;
      const llamaId = llama.status === "fulfilled" ? llama.value.loaded_model_id : null;
      // Reconcile the run state with the backend, not only the chip. A server
      // that died after it was reported running would otherwise keep the
      // terminal pinned forever, because nothing else re-reads this while no
      // load is in flight.
      if (llama.status === "fulfilled") {
        const status = llama.value;
        setLocalRuntime((current) =>
          current.loaded_model_id === status.loaded_model_id
            && current.loading_model_id === status.loading_model_id
            && current.started_at === status.started_at
            ? current
            : { ...current, loaded_model_id: status.loaded_model_id, loading_model_id: status.loading_model_id, started_at: status.started_at },
        );
      }
      const llamaName = llamaId ? await invoke<{ id: string; name: string }[]>("local_models_list")
        .then((models) => models.find((model) => model.id === llamaId)?.name ?? null)
        .catch(() => null) : null;
      // One audio process per purpose: a purpose that is not answering keeps
      // its last figure rather than blinking out of the chip, and a purpose
      // that answered "not running" really is gone.
      const resident = (result: typeof speech | typeof wake) =>
        result.status === "fulfilled" && result.value.running
          ? { model: result.value.loaded, startedAt: result.value.startedAt }
          : null;
      setResidentServers((current) => ({
        llama: llamaName ?? (llamaId ? current.llama : null),
        llamaStartedAt: llama.status === "fulfilled" && llamaId ? llama.value.started_at : null,
        speech: speech.status === "fulfilled" ? resident(speech) : current.speech,
        wake: wake.status === "fulfilled" ? resident(wake) : current.wake,
      }));
    };
    void poll();
    const timer = window.setInterval(() => { void poll(); }, 4000);
    return () => {
      live = false;
      window.clearInterval(timer);
    };
  }, []);

  /** Stops one audio engine from its own header row; the next poll confirms. */
  const stopAudioServer = async (purpose: "speech" | "wake") => {
    try {
      await invoke("audio_stop", { purpose });
      setResidentServers((current) => ({ ...current, [purpose]: null }));
    } catch (reason) {
      notify("error", String(reason));
    }
  };

  const retryStartup = () => {
    setStartupError("");
    setStartupProgress({ phase: "database", message: "Opening database", progress: 4 });
    void invoke("database_initialize").then(() => chatRepository.initialize()).then(() => loadGeneralSettings().catch(() => undefined)).then(() => {
      setStartupDone(true);
      setSessionReloadKey((key) => key + 1);
    }).catch((reason: unknown) => setStartupError(String(reason)));
  };

  if (!startupDone || !chat.loaded || chat.loadError) return <StartupScreen message={startupProgress.message} progress={startupProgress.progress} error={startupError || chat.loadError} onRetry={retryStartup} />;

  return (
    <div className="flex h-dvh min-w-0 overflow-hidden bg-[var(--page)] text-[var(--text)]">
      <Sidebar
        sessions={chat.sessions}
        activeId={chat.activeId}
        activity={sessionActivity}
        onSelectSession={handleSelectSession}
        onNewChat={() => chat.selectSession(null)}
        onDeleteSession={chat.deleteSession}
        onRenameSession={chat.renameSession}
        onSetDotColor={chat.setSessionDotColor}
        onTogglePinned={chat.togglePinned}
        onOpenOverview={handleOpenOverview}
        onSettings={() => setSettingsOpen(true)}
        view={view}
        onViewChange={setView}
      />
      {view === "database" && <div className="flex min-w-0 flex-1 flex-col"><DatabasePage refreshKey={endpointRefreshKey} /></div>}
      {view === "jobs" && <div className="flex min-w-0 flex-1 flex-col"><JobsPage notify={notify} onOpenSession={(id) => { void chat.selectSession(id); setView("chat"); }} /></div>}
      {view === "statistics" && <div className="flex min-w-0 flex-1 flex-col"><Suspense fallback={<PageFallback />}><StatisticsPage refreshKey={endpointRefreshKey} sessions={chat.sessions} onOpenOverview={handleOpenOverview} /></Suspense></div>}
      <div className={`min-w-0 flex-1 flex-col ${view === "chat" ? "flex" : "hidden"}`}>
        {view === "chat" && <>
        <header className="relative z-30 flex min-h-[60px] items-center justify-between gap-4 border-b border-[var(--line)] bg-[var(--page)] px-5 sm:px-7">
          <div className="min-w-0">
            {/* The folder name is the trigger, and nothing else is.
                A second row reading "Change folder" said nothing the name did
                not, and left the reader to work out which of two lines was the
                control. One row: the name, a caret for affordance, and the
                hover tint. With nothing open it reads "Open a folder", which is
                both the label and the action. */}
            <div ref={workspaceMenuRef} className="relative">
              <button
                type="button"
                onClick={() => setWorkspaceMenuOpen((open) => !open)}
                aria-expanded={workspaceMenuOpen}
                aria-haspopup="menu"
                aria-label={workspaces.selected ? `Workspace: ${workspaceLabel(workspaces.selected)}. Change folder` : "Choose a workspace folder"}
                title={workspaces.selected ?? "No folder open"}
                className="-mx-1 flex max-w-full items-center gap-1 rounded px-1 text-[11px] text-[var(--muted)] transition-colors hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
              >
                <span className="truncate">
                  {workspaces.selected ? workspaceLabel(workspaces.selected) : "Open a folder"}
                </span>
                <ChevronDown size={12} className="shrink-0 text-[var(--quiet)]" />
              </button>
              {workspaceMenuOpen && (
                <WorkspacePicker
                  workspaces={workspaces}
                  onOpen={() => { void chooseFolder().then(() => setWorkspaceMenuOpen(false)); }}
                  onSelect={(path) => { void selectWorkspace(path).then(() => setWorkspaceMenuOpen(false)); }}
                  onClose={(path) => { void closeWorkspace(path); }}
                  onDismiss={() => setWorkspaceMenuOpen(false)}
                />
              )}
            </div>
            <h1 className="truncate text-sm font-medium">{chat.activeSession?.title ?? "New conversation"}</h1>
          </div>
          <div className="min-w-0 flex-1" />
          {workspaceError && <span className="max-w-56 truncate text-[11px] text-[var(--danger)]">{workspaceError}</span>}
          {(residentServers.llama !== null || residentServers.speech !== null || residentServers.wake !== null) && (
            <div className="relative">
              <button
                ref={serversChipRef}
                type="button"
                onClick={() => {
                  const box = serversChipRef.current?.getBoundingClientRect();
                  if (box) {
                    // The menu is `fixed`, so the chip's viewport rect is the
                    // menu's viewport anchor -- measured at open, so the right
                    // panel squeezing this header, a resize, or a wrapped title
                    // never strand it at one corner.
                    setServersMenu({ top: Math.round(box.bottom + 6), right: Math.round(window.innerWidth - box.right) });
                  }
                  setServersMenuOpen((open) => !open);
                }}
                aria-expanded={serversMenuOpen}
                aria-haspopup="menu"
                title="What is resident right now"
                className="inline-flex min-h-9 items-center gap-2 rounded-md border border-[var(--line)] bg-[var(--panel)] px-3 text-xs font-medium text-[var(--muted)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
              >
                <ChevronDown size={14} className={`transition-transform ${serversMenuOpen ? "rotate-180" : ""}`} />
                {[
                  residentServers.llama ? displayModelName(residentServers.llama) : null,
                  residentServers.speech ? residentServers.speech.model ?? "audio.cpp" : null,
                  residentServers.wake ? residentServers.wake.model ?? "audio.cpp" : null,
                ]
                  .filter((label): label is string => Boolean(label))
                  .filter((label, index, all) => all.indexOf(label) === index)
                  .join(" · ")}
              </button>
              {/* The menu is portaled to the document body and pinned by the
                  chip's own measured rect: the chat page directly below this
                  header is its own stacking layer, and a floating panel that
                  lives inside the header's subtree can lose the paint order
                  there. A portaled `fixed` panel cannot. */}
              {serversMenuOpen && createPortal(
                <>
                  {/* Click-away, under the menu, the way every other menu closes. */}
                  <button type="button" aria-label="Close" tabIndex={-1} onClick={() => setServersMenuOpen(false)} className="fixed inset-0 z-[60] cursor-default" />
                  <div
                    role="menu"
                    style={{ top: serversMenu.top, right: serversMenu.right }}
                    className="fixed z-[70] w-72 rounded-lg border border-[var(--line)] bg-[var(--panel)] p-1 shadow-lg"
                  >
                    {residentServers.llama && (
                      <div role="menuitem" className="flex min-h-10 items-center gap-3 rounded-md px-2.5 py-1.5">
                        <div className="min-w-0 flex-1">
                          <h4 className="truncate text-xs font-medium text-[var(--text)]">{residentServers.llama}</h4>
                          <p className="truncate text-[11px] text-[var(--quiet)]">
                            llama-server · loaded
                            {residentServers.llamaStartedAt !== null && <ResidentFor startedAt={residentServers.llamaStartedAt} />}
                          </p>
                        </div>
                        <button
                          type="button"
                          onClick={() => { void unloadLocalModel(); setServersMenuOpen(false); }}
                          className="inline-flex min-h-7 shrink-0 items-center gap-1 rounded px-2 text-[11px] text-[var(--text)] hover:bg-[var(--raised)] hover:text-[var(--danger)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
                        >
                          <Power size={13} />
                          Unload
                        </button>
                      </div>
                    )}
                    {([["speech", residentServers.speech], ["wake", residentServers.wake]] as const).map(
                      ([purpose, entry]) =>
                        entry && (
                          <div key={purpose} role="menuitem" className="flex min-h-10 items-center gap-3 rounded-md px-2.5 py-1.5">
                            <div className="min-w-0 flex-1">
                              <h4 className="truncate text-xs font-medium text-[var(--text)]">
                                {entry.model ?? "audio.cpp"}
                              </h4>
                              <p className="truncate text-[11px] text-[var(--quiet)]">
                                audio.cpp · {purpose} · running
                                {entry.startedAt !== null && <ResidentFor startedAt={entry.startedAt} />}
                              </p>
                            </div>
                            <button
                              type="button"
                              onClick={() => { void stopAudioServer(purpose); setServersMenuOpen(false); }}
                              className="inline-flex min-h-7 shrink-0 items-center gap-1 rounded px-2 text-[11px] text-[var(--text)] hover:bg-[var(--raised)] hover:text-[var(--danger)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
                            >
                              <Power size={13} />
                              Stop
                            </button>
                          </div>
                        ),
                    )}
                  </div>
                </>,
                document.body,
              )}
            </div>
          )}
        </header>
        <ChatPage
          key="chat-page"
          endpointId={endpointId}
          model={model}
          localModelId={localModelId}
          session={chat.activeSession}
          activeId={chat.activeId}
          sessionLoading={chat.activeId !== null && !chat.activeLoaded}
          onEnsureSession={ensureSession}
          onFirstSend={() => setView("chat")}
          onActivity={handleSessionActivity}
          onModelSelect={(id, selectedModel) => { setEndpointId(id); setModel(selectedModel); setLocalModelId(""); }}
          onLocalModelSelect={(id) => {
            setLocalModelId(id);
            void invoke("local_model_select", { id });
          }}
          modelRefreshKey={endpointRefreshKey}
          onMessagesChange={handleMessagesChange}
          onWorkingModeChange={chat.setSessionWorkingMode}
          onNotify={notify}
          onNotifyAction={notifyAction}
          onClearNotification={clearNotification}
          onOpenUrl={(url) => setRequestedUrl({ url, key: Date.now() })}
          onOpenFile={(path) => {
            // A file clicked in a reply belongs to the workspace. If the panel is
            // still showing a skill folder from Settings, the request would be
            // resolved against the skills root -- which is how a document named
            // `NOTES.xlsx` came back as a missing file under `skills/`. Clearing
            // the skill is the same switch the × in that header performs.
            setOpenSkill(null);
            setRequestedFile(path);
          }}
          titleModels={preferences.sessionTitleModels}
          onAutoTitle={chat.setAutoTitle}
          workspaceRoot={workspaces.selected}
          defaultEffort={preferences.mainReasoning}
        />
        </>}
      </div>

      {/* Last in the row, so it takes width from the chat column rather than
          pushing the sidebar off-screen. `shrink-0` is what lets it hold a fixed
          width while the transcript flexes around it. */}
      <RightPanel
        open={rightPanel.open}
        width={rightPanel.width}
        workspace={workspaces.selected}
        skill={openSkill}
        onCloseSkill={() => setOpenSkill(null)}
        sessionId={chat.activeId}
        loadedModelId={localRuntime.loaded_model_id}
        loadingModelId={localRuntime.loading_model_id}
        terminalRequest={terminalRequest}
        requestedPath={requestedFile}
        requestedUrl={requestedUrl}
        onUrlRequestHandled={() => setRequestedUrl(null)}
        onRequestHandled={() => setRequestedFile(null)}
        overviewRequest={overviewRequest}
        loadSession={chat.loadSessionForOverview}
        onClose={() => rightPanel.setOpen(false)}
        onOpen={() => rightPanel.setOpen(true)}
        onResize={rightPanel.setWidth}
        onNotify={notify}
      />

      <NotificationStack notifications={notifications} copiedId={copiedNotificationId} onDismiss={dismissNotification} onCopy={copyNotification} />
      {/* One find bar for the whole app. It portals itself into the settings
          dialog when that is open, and searches whichever surfaces declared
          themselves scopes -- so Ctrl+F means the same thing on every view. */}
      <FindBar />
      <SettingsModal open={settingsOpen} onClose={() => setSettingsOpen(false)} onEndpointsChanged={() => setEndpointRefreshKey((key) => key + 1)} preferences={preferences} onPreferencesChange={savePreferences} appearance={appearance} onAppearanceChange={saveAppearance} notify={notify} onOpenSkill={(slug) => { setOpenSkill(slug); setSettingsOpen(false); }} />
      <TooltipLayer />
    </div>
  );
}

