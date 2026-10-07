import { Suspense, lazy, useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { ChevronDown, Power } from "lucide-react";
import NotificationStack from "./features/notifications/NotificationStack";
import { NotificationHostProvider } from "./features/notifications/notificationHost";
import { useCopiedNotification, useNotifications } from "./features/notifications/useNotifications";
import Sidebar from "./components/Sidebar";
import ChatPage from "./pages/chat/ChatPage";
import SettingsModal from "./components/SettingsModal";
import StartupScreen from "./components/StartupScreen";
import DatabasePage from "./pages/DatabasePage";
import SessionOverview from "./components/SessionOverview";
import WorkspacePicker from "./components/WorkspacePicker";
import RightPanel from "./components/rightPanel/RightPanel";
import { useChatSessions } from "./features/chat/useChatSessions";
import { chatRepository } from "./features/chat/chatRepository";
import { usePreferences } from "./features/models/usePreferences";
import { useRightPanel } from "./features/rightPanel/useRightPanel";
import { useWorkspaces, workspaceLabel } from "./features/workspace/useWorkspaces";
import type { ChatMessage, ChatSession, SessionActivity, SessionRunStatus } from "./features/chat/types";
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

type LocalRuntimeStatus = { loaded_model_id: string | null; loading_model_id: string | null; selected_model_id: string | null; base_url: string };
type LocalModelEvent = { kind: "loading" | "loaded" | "failed" | "unloaded"; modelId?: string; modelPath?: string; message?: string };
type StartupProgress = { phase: string; message: string; progress: number };

/**
 * The host provider sits above the shell so a dialog opened anywhere in the tree
 * can claim the notification stack's portal target -- see
 * `features/notifications/notificationHost.tsx` for why a `showModal()` dialog
 * needs this at all.
 */
export default function App() {
  return (
    <NotificationHostProvider>
      <AppShell />
    </NotificationHostProvider>
  );
}

function AppShell() {
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsRefreshKey, setSettingsRefreshKey] = useState(0);
  const [startupDone, setStartupDone] = useState(false);
  const [startupError, setStartupError] = useState("");
  const [sessionReloadKey, setSessionReloadKey] = useState(0);
  const [startupProgress, setStartupProgress] = useState<StartupProgress>({ phase: "database", message: "Opening database", progress: 4 });
  const [view, setView] = useState<"chat" | "database" | "statistics">("chat");
  const [endpointId, setEndpointId] = useState("");
  const [model, setModel] = useState("");
  const [localModelId, setLocalModelId] = useState("");
  const [endpointRefreshKey, setEndpointRefreshKey] = useState(0);
  const [localRuntime, setLocalRuntime] = useState<LocalRuntimeStatus>({ loaded_model_id: null, loading_model_id: null, selected_model_id: null, base_url: "http://127.0.0.1:43127/v1" });
  // The corner stack lives in its own module rather than here, so a view like the
  // settings modal can be handed a `notify` without the stack's shape being a
  // detail of the app shell. `App` renders it once and owns the wiring.
  const { notifications, notify, dismiss: dismissNotification, clearKey, clearWhere } = useNotifications();
  const { copiedId: copiedNotificationId, markCopied } = useCopiedNotification();
  const [sessionActivity, setSessionActivity] = useState<SessionActivity>({});
  const [overviewSession, setOverviewSession] = useState<ChatSession | null>(null);
  const chat = useChatSessions(startupDone, sessionReloadKey, localModelId || model, endpointId);
  // Preferences live here so the chat view and the settings modal always agree
  // on the title-model choice without one re-reading a stale copy.
  const { preferences, savePreferences } = usePreferences(startupDone);
  const rightPanel = useRightPanel();
  // The file the transcript asked the panel to show, and the acknowledgement that
  // clears it. Held here rather than inside `RightPanel` because the request comes
  // from the chat column beside it, and the two must not hold references to each
  // other to pass one string.
  const [requestedFile, setRequestedFile] = useState<string | null>(null);
  const [requestedUrl, setRequestedUrl] = useState<{ url: string; key: number } | null>(null);
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
        if (sessionId === activeIdRef.current) return current;
        if (current[sessionId] === status) return current;
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
   * Open the read-only overview for a conversation.
   *
   * Owned here rather than in the sidebar or the statistics page because the
   * overview needs the transcript, and transcripts are fetched per conversation
   * rather than all at startup. One owner means one fetch and one modal, so the
   * two entry points cannot drift into showing different figures for the same
   * conversation. Fetching is skipped when the transcript is already in memory.
   */
  const handleOpenOverview = useCallback(async (id: string) => {
    const session = await chat.loadSessionForOverview(id);
    if (session) setOverviewSession(session);
  }, [chat.loadSessionForOverview]);

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
    void Promise.all([
      listen<StartupProgress>("startup-progress", (event) => setStartupProgress(event.payload)).then((unlisten) => {
        if (mounted) unlistenProgress = unlisten;
        else unlisten();
      }),
      listen<LocalModelEvent>("local-model-event", (event) => {
        const payload = event.payload;
        if (payload.kind === "loading") {
          setLocalRuntime((current) => ({ ...current, loading_model_id: payload.modelId ?? null }));
          notify("loading", "Loading local model…", "local-model-loading");
        } else if (payload.kind === "loaded") {
          setLocalRuntime((current) => ({ ...current, loaded_model_id: payload.modelId ?? null, loading_model_id: null }));
          clearKey("local-model-loading");
          notify("success", "Local model ready");
        } else if (payload.kind === "unloaded") {
          setLocalRuntime((current) => ({ ...current, loaded_model_id: null, loading_model_id: null }));
          clearWhere((notification) => notification.key?.startsWith("local-model:") === true);
        } else {
          setLocalRuntime((current) => ({ ...current, loaded_model_id: null, loading_model_id: null }));
          clearKey("local-model-loading");
          notify("error", payload.message || "Local model failed to load");
        }
      }).then((unlisten) => {
        if (mounted) unlistenLocal = unlisten;
        else unlisten();
      }),
    ]).then(() => mounted ? invoke("database_initialize") : Promise.reject(new Error("Startup cancelled"))).then(() => {
      setStartupProgress({ phase: "migration", message: "Importing existing data", progress: 94 });
      return chatRepository.initialize();
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
    };
  }, [clearKey, notify]);

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
      setLocalRuntime((current) => ({ ...current, loaded_model_id: null, loading_model_id: null }));
      clearWhere((notification) => notification.key?.startsWith("local-model:") === true);
    } catch (reason) {
      clearKey("local-model-loading");
      notify("error", String(reason));
    }
  };

  const retryStartup = () => {
    setStartupError("");
    setStartupProgress({ phase: "database", message: "Opening database", progress: 4 });
    void invoke("database_initialize").then(() => chatRepository.initialize()).then(() => {
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
      {view === "statistics" && <div className="flex min-w-0 flex-1 flex-col"><Suspense fallback={<PageFallback />}><StatisticsPage refreshKey={endpointRefreshKey} sessions={chat.sessions} onOpenOverview={handleOpenOverview} /></Suspense></div>}
      <div className={`min-w-0 flex-1 flex-col ${view === "chat" ? "flex" : "hidden"}`}>
        {view === "chat" && <>
        <header className="flex min-h-[68px] items-center justify-between gap-4 border-b border-[var(--line)] bg-[var(--page)] px-5 sm:px-7">
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
                className="-mx-1 flex max-w-full items-center gap-1 rounded px-1 text-sm text-[var(--muted)] transition-colors hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
              >
                <span className="truncate">
                  {workspaces.selected ? workspaceLabel(workspaces.selected) : "Open a folder"}
                </span>
                <ChevronDown size={13} className="shrink-0 text-[var(--quiet)]" />
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
          {localRuntime.loaded_model_id && <button type="button" onClick={() => void unloadLocalModel()} className="inline-flex min-h-9 items-center gap-2 rounded-md border border-[var(--line)] bg-[var(--panel)] px-3 text-xs font-medium text-[var(--muted)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)]"><Power size={14} />Unload model</button>}
        </header>
        <ChatPage
          key="chat-page"
          endpointId={endpointId}
          model={model}
          localModelId={localModelId}
          session={chat.activeSession}
          activeId={chat.activeId}
          sessionLoading={chat.activeId !== null && !chat.activeLoaded}
          onOpenSettings={() => setSettingsOpen(true)}
          onEnsureSession={ensureSession}
          onFirstSend={() => setView("chat")}
          onActivity={handleSessionActivity}
          onModelSelect={(id, selectedModel) => { setEndpointId(id); setModel(selectedModel); setLocalModelId(""); }}
          onLocalModelSelect={(id) => {
            setLocalModelId(id);
            void invoke("local_model_select", { id });
          }}
          modelRefreshKey={endpointRefreshKey}
          settingsRefreshKey={settingsRefreshKey}
          onMessagesChange={handleMessagesChange}
          onWorkingModeChange={chat.setSessionWorkingMode}
          onNotify={notify}
          onOpenUrl={(url) => setRequestedUrl({ url, key: Date.now() })}
          onOpenFile={setRequestedFile}
          titleModels={preferences.sessionTitleModels}
          onAutoTitle={chat.setAutoTitle}
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
        sessionId={chat.activeId}
        loadedModelId={localRuntime.loaded_model_id}
        loadingModelId={localRuntime.loading_model_id}
        requestedPath={requestedFile}
        requestedUrl={requestedUrl}
        onUrlRequestHandled={() => setRequestedUrl(null)}
        onRequestHandled={() => setRequestedFile(null)}
        onClose={() => rightPanel.setOpen(false)}
        onOpen={() => rightPanel.setOpen(true)}
        onResize={rightPanel.setWidth}
        onNotify={notify}
      />

      <NotificationStack notifications={notifications} copiedId={copiedNotificationId} onDismiss={dismissNotification} onCopy={copyNotification} />
      <SettingsModal open={settingsOpen} onClose={() => { setSettingsOpen(false); setSettingsRefreshKey((key) => key + 1); }} onEndpointsChanged={() => setEndpointRefreshKey((key) => key + 1)} onClearSessions={chat.clearSessions} preferences={preferences} onPreferencesChange={savePreferences} notify={notify} />
      {/* One overview for both entry points, portalled so the overlay sits above
          the app rather than inside whichever column raised it. Loaded eagerly:
          unlike the statistics page it pulls in no chart library, and the modal
          is reachable from the sidebar on every screen. */}
      {overviewSession && createPortal(<SessionOverview session={overviewSession} onClose={() => setOverviewSession(null)} />, document.body)}
    </div>
  );
}
