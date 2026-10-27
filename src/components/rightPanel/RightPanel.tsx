import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { PanelRightClose } from "lucide-react";
import PanelTabs from "./PanelTabs";
import RightPanelEmpty from "./RightPanelEmpty";
import BrowserTab from "./browser/BrowserTab";
import FilesTab from "./tabs/FilesTab";
import TerminalTab from "./tabs/TerminalTab";
import SubAgentsTab from "./tabs/SubAgentsTab";
import SessionOverviewTab from "./tabs/SessionOverviewTab";
import type { ChatSession } from "../../features/chat/types";
import { type PanelViewId, usePanelViews } from "../../features/rightPanel/usePanelViews";
import { MAX_PANEL_WIDTH, MIN_PANEL_WIDTH } from "../../features/rightPanel/useRightPanel";

/**
 * The drag surface on the panel's left edge.
 *
 * Its own component because it holds the only pointer-capture state in the panel,
 * and because it must keep rendering when the panel is closed -- a panel at zero
 * width with no handle has no way back open, so the handle lives in the shell
 * rather than inside the body.
 *
 * **The drag never re-renders the app.** `onDrag` writes the width straight to
 * the panel's inline style and `onCommit` reports it once, on release. Routing
 * every pointer move through React state re-rendered the entire tree -- chat
 * transcript, sidebar, notification stack -- sixty times a second to move one
 * box, which is what made the drag feel like it was dragging something. The
 * committed value still goes through `onResize` exactly as before, so persistence
 * and the keyboard affordances are unchanged.
 *
 * **The cursor is set on the document, not on the handle.** A capture keeps the
 * pointer events coming even when the cursor leaves the handle, but the browser
 * still draws the cursor wherever the real pointer is. Without this the resize
 * reads as a drag that stopped working halfway across the window.
 */
function ResizeHandle({
  width,
  onDrag,
  onCommit,
  panel,
}: {
  width: number;
  /** Applies a width with no React render. Fires continuously during a drag. */
  onDrag: (width: number) => void;
  /** Reports the finished width. Fires once, on release. */
  onCommit: (width: number) => void;
  /** The panel element, so the drag can switch its width transition off. */
  panel: React.RefObject<HTMLElement | null>;
}) {
  const [dragging, setDragging] = useState(false);
  const startRef = useRef({ pointerX: 0, width: 0 });

  const clamp = useCallback((next: number) => Math.min(MAX_PANEL_WIDTH, Math.max(MIN_PANEL_WIDTH, next)), []);

  const handlePointerDown = useCallback((event: React.PointerEvent<HTMLDivElement>) => {
    // Primary button only: a right-click drag here would otherwise resize the
    // panel, and the context menu the user asked for never appears.
    if (event.button !== 0) return;
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    startRef.current = { pointerX: event.clientX, width };
    // Off for the drag, back on at commit. Without this the 200ms ease applies
    // to every one of the sixty width values a second-long drag produces, so the
    // panel is always a fifth of a second behind the pointer -- which reads as
    // the panel being heavy rather than the easing being wrong.
    if (panel.current) panel.current.style.transition = "none";
    setDragging(true);
  }, [panel, width]);

  const handlePointerMove = useCallback((event: React.PointerEvent<HTMLDivElement>) => {
    if (!event.currentTarget.hasPointerCapture(event.pointerId)) return;
    // The panel grows leftwards, so the delta is negated: dragging the edge left
    // must widen it, which is what the user sees on every other resizable edge
    // in the app.
    onDrag(clamp(startRef.current.width + (startRef.current.pointerX - event.clientX)));
  }, [clamp, onDrag]);

  const endDrag = useCallback((event: React.PointerEvent<HTMLDivElement>) => {
    if (!event.currentTarget.hasPointerCapture(event.pointerId)) return;
    event.currentTarget.releasePointerCapture(event.pointerId);
    setDragging(false);
    // Reported from the handler rather than from the last `pointermove`, so a
    // drag that ends without a final move -- the pointer lifted on the same pixel
    // it started on -- still persists the width it settled at.
    onCommit(clamp(startRef.current.width + (startRef.current.pointerX - event.clientX)));
  }, [clamp, onCommit]);

  useEffect(() => {
    if (!dragging) return;
    const previous = document.body.style.cursor;
    const previousSelect = document.body.style.userSelect;
    document.body.style.cursor = "col-resize";
    // Text selection during a drag is what makes a resize feel broken: the
    // cursor drags a highlight across the transcript instead of the edge.
    document.body.style.userSelect = "none";
    return () => {
      document.body.style.cursor = previous;
      document.body.style.userSelect = previousSelect;
    };
  }, [dragging]);

  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label="Resize panel"
      aria-valuenow={Math.round(width)}
      aria-valuemin={MIN_PANEL_WIDTH}
      aria-valuemax={MAX_PANEL_WIDTH}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={endDrag}
      onPointerCancel={endDrag}
      // A wider invisible band than it looks: 8px is below the ~9px comfortable
      // target, and a resize edge that is hard to grab is a resize edge nobody
      // finds. The visible line stays hairline; only the hit area is generous.
      className="group absolute inset-y-0 -left-1 z-10 w-2 cursor-col-resize"
    >
      {/* The line itself, lifting to the accent on hover and while dragging so
          the edge announces itself without adding a permanent border down the
          whole app. */}
      <div className={`h-full w-px transition-colors ${dragging ? "bg-[var(--accent)]" : "bg-transparent group-hover:bg-[var(--line)]"}`} />
    </div>
  );
}

type RightPanelProps = {
  open: boolean;
  width: number;
  /** The open workspace path, handed to the views that need it. */
  workspace: string | null;
  /**
   * An installed skill the Files view should draw instead of the workspace.
   *
   * Set by the Skills page when a skill is double-clicked. It is a *request* in
   * the same sense as `requestedPath`: setting it opens the panel and brings
   * Files forward, and the × in that view's header calls `onCloseSkill` to give
   * the workspace tree back.
   */
  skill?: string | null;
  /** Clears `skill`, returning the Files view to the workspace. */
  onCloseSkill?: () => void;
  /**
   * The conversation on screen, so the Sub agents view can follow it.
   *
   * `null` for a new, unsaved chat: the view then lists every run rather than
   * pretending none exist, because the runs a chat spawned outlive any one view
   * of it and are worth reaching even before the chat is named.
   */
  sessionId?: string | null;
  loadedModelId?: string | null;
  loadingModelId?: string | null;
  /**
   * A request to show the model terminal, raised on every load attempt.
   *
   * Separate from `loadingModelId` because a failed start can clear that value
   * before this panel renders -- the two events are emitted in one tick -- and
   * the terminal is exactly what a user wants to read after a failure.
   */
  terminalRequest?: { id: string; key: number } | null;
  /**
   * A file to show in the Files view, from outside the panel.
   *
   * A request rather than a selection, and it opens the panel and brings Files
   * forward on its own -- so a caller that only sets this does not also have to
   * remember to open anything. It is cleared by `onRequestHandled` once consumed,
   * so the panel does not keep reopening the last file anyone clicked.
   */
  requestedPath?: string | null;
  requestedUrl?: { url: string; key: number } | null;
  onUrlRequestHandled?: () => void;
  /** Called once the requested file has been shown. */
  onRequestHandled?: () => void;
  /**
   * The conversation the Overview view should show, raised by the sidebar or the
   * statistics page.
   *
   * A request rather than a selection, like `requestedPath`: setting it opens the
   * panel and brings Overview forward on its own. The `key` changes on every
   * request, so asking for the same conversation again still re-opens the view
   * rather than leaving a closed panel untouched.
   */
  overviewRequest?: { id: string; key: number } | null;
  /**
   * Reads one conversation with its transcript, for the Overview view.
   *
   * Supplied by the chat hook so the panel shares its transcript cache rather
   * than holding a second copy of every conversation the user looks at.
   */
  loadSession: (id: string) => Promise<ChatSession | null>;
  onClose: () => void;
  onOpen: () => void;
  onResize: (width: number) => void;
  /** Reported so a refused address reaches the app's one notification stack. */
  onNotify?: (tone: "success" | "error", message: string) => void;
};

/**
 * The collapsible right-hand dock.
 *
 * Closed is *zero width*, not `display: none`, so the handle can stay on screen
 * to bring it back. The body is hidden from the tree as well as from layout, so
 * a tab the panel remembers is not silently live behind a closed panel.
 */
export default function RightPanel({ open, width, workspace, skill = null, onCloseSkill, sessionId = null, loadedModelId = null, loadingModelId = null, terminalRequest = null, requestedPath, requestedUrl, onUrlRequestHandled, onRequestHandled, overviewRequest = null, loadSession, onClose, onOpen, onResize, onNotify }: RightPanelProps) {
  const panelRef = useRef<HTMLElement>(null);
  const {
    openViews, activeView, open: openView, close: closeView, setActiveView,
    terminalSessions, activeTerminalId, setActiveTerminalId, openTerminalSession,
    closeTerminalSession, closeAll: closeAllViews, ensureServerSession,
  } = usePanelViews();
  const serverModelId = loadedModelId ?? loadingModelId;
  // A file:// URL asked for by the Files view. Object rather than a bare string
  // so opening the same file twice still re-navigates -- the nonce changes.
  const [browserRequest, setBrowserRequest] = useState<{ url: string; nonce: number } | null>(null);
  const handledUrlRequestKeyRef = useRef<number | null>(null);

  // The last server model this effect acted on. Used to avoid re-running the
  // side effects when `onOpen` or other parent callbacks change identity on
  // every render -- which would snap the active view back to the terminal on
  // every render while a local server is running.
  const lastServerModelIdRef = useRef<string | null>(null);

  /**
   * The llama session's title, from the model's own name.
   *
   * A short id (`3f9a1c2b`) names nothing a reader recognises; the file's name
   * is what the Local models page showed them, so the terminal says the same
   * thing. The id is the fallback for a model deleted while its server runs.
   */
  const titleForModel = useCallback(
    (id: string): Promise<string> =>
      invoke<{ id: string; name: string }[]>("local_models_list")
        .then((models) => {
          const name = models.find((model) => model.id === id)?.name;
          return `llama-server · ${name ?? id.slice(0, 8)}`;
        })
        .catch(() => `llama-server · ${id.slice(0, 8)}`),
    [],
  );

  useEffect(() => {
    if (!serverModelId) {
      lastServerModelIdRef.current = null;
      return;
    }
    // Only act when the *model* changes. A re-render that merely handed us a
    // new `onOpen` identity must not steal focus back from whatever the user
    // opened in the meantime.
    if (lastServerModelIdRef.current === serverModelId) return;
    lastServerModelIdRef.current = serverModelId;
    void titleForModel(serverModelId).then((title) =>
      ensureServerSession(serverModelId, title),
    );
    openView("terminal");
    onOpen();
  }, [serverModelId, ensureServerSession, titleForModel, openView, onOpen]);

  // Every attempt, including one whose start is refused at once. The request
  // carries its own key so two attempts at the same model both act, and a
  // re-render is not mistaken for a new attempt.
  const handledTerminalKeyRef = useRef<number | null>(null);
  useEffect(() => {
    if (!terminalRequest) return;
    if (handledTerminalKeyRef.current === terminalRequest.key) return;
    handledTerminalKeyRef.current = terminalRequest.key;
    void titleForModel(terminalRequest.id).then((title) =>
      ensureServerSession(terminalRequest.id, title),
    );
    openView("terminal");
    onOpen();
  }, [terminalRequest, ensureServerSession, titleForModel, openView, onOpen]);

  // The audio servers, learned the same way the llama server is -- from a
  // prop-like source that answers on mount. Only a ref is kept: what the panel
  // needs is "did this purpose change since the last event" so the terminal is
  // not reopened on every heartbeat, not a value anything draws. Two purposes,
  // so one entry each.
  const audioRunningRef = useRef<Record<string, boolean>>({});

  type AudioPurpose = "speech" | "wake";

  /** The audio session ids, and the purpose each one's process serves. */
  const isAudioSession = (id: string) => id === "audiocpp" || id === "audiocpp-wake";
  const audioPurpose = (id: string): AudioPurpose =>
    id === "audiocpp-wake" ? "wake" : "speech";

  /**
   * One audio engine's session, named after the model it is holding.
   *
   * There are two engines -- speech and wake -- and each is its own process
   * with its own resident model, so each gets its own row. Calling either one
   * `audiocpp_server` said nothing a reader wants: the resident model --
   * `open wake word`, `kokoro tts` -- is what they actually started.
   * `ensureServerSession` updates an existing row's title, so this also renames
   * the row a later load made.
   */
  const ensureAudioSession = useCallback(
    async (purpose: AudioPurpose) => {
      try {
        const status = await invoke<{ running: boolean; loaded: string | null }>("audio_status", {
          purpose,
        });
        if (status.running) {
          ensureServerSession(
            purpose === "wake" ? "audiocpp-wake" : "audiocpp",
            `audio.cpp · ${status.loaded ?? "loading"}`,
          );
        }
      } catch {
        // A status that cannot be read is not worth a row nobody asked for.
      }
    },
    [ensureServerSession],
  );

  // The mount probe is what makes a session survive a closed panel: the start
  // event fired while this component was still mounted, but the *panel* was
  // closed, and on reopen nothing would otherwise re-ensure the sessions.
  // Asking the backend once per mount, per purpose, covers every reopen.
  useEffect(() => {
    void Promise.all([ensureAudioSession("speech"), ensureAudioSession("wake")]).catch(
      () => undefined,
    );
  }, [ensureAudioSession]);

  useEffect(() => {
    let live = true;
    const unlisten = listen<{ running: boolean; model: string; engine?: string }>(
      "wake-state",
      ({ payload }) => {
        if (!live || !payload.running) return;
        // A pack runs in the audio.cpp wake process, whose own start event is
        // what makes its row. Only the ONNX path has no process, so only it
        // needs a session of its own -- otherwise one engine appeared twice.
        if (payload.engine !== "openwakeword") return;
        ensureServerSession("openwakeword", `wake · ${payload.model}`);
      },
    );
    return () => {
      live = false;
      void unlisten.then((stop) => stop());
    };
  }, [ensureServerSession]);

  useEffect(() => {
    let live = true;
    const unlisten = listen<{ running: boolean; purpose?: string }>(
      "audio-server-state",
      ({ payload }) => {
        if (!live) return;
        const purpose: AudioPurpose = payload.purpose === "wake" ? "wake" : "speech";
        if (payload.running === audioRunningRef.current[purpose]) return;
        audioRunningRef.current[purpose] = payload.running;
        if (!payload.running) return;
        // The event says it started, not what it is holding; the title is
        // filled in from the status a moment later. The row is made here but
        // the terminal is not opened: a server starting is not a request to
        // read its log, and two purposes meant two tabs opened unasked.
        void ensureAudioSession(purpose);
      },
    );
    return () => {
      live = false;
      void unlisten.then((stop) => stop());
    };
  }, [ensureAudioSession]);

  // A server terminal stays open for as long as its process answers so its
  // output is not lost behind a closed tab -- but closing the Terminal view is a
  // deliberate "I am done with this", so it stops what the view was showing
  // rather than refusing. A failed engine that never produced a healthy process
  // used to leave the tab impossible to close, which is the state this fixes.

  /**
   * Closing a server's terminal row stops the process it was showing.
   *
   * The row is the process's only on-screen identity, so its close is where
   * the stop belongs -- a remote log with the process still running would be
   * two answers to one question. Only a confirmed stop closes the session; a
   * failed one is reported and the row stays so the user still sees the
   * process's last words.
   */
  const handleCloseTerminal = useCallback((id: string) => {
    const session = terminalSessions.find((entry) => entry.id === id);
    if (!session) return;
    if (session.kind !== "server") {
      closeTerminalSession(id);
      return;
    }
    const stopAndClose = isAudioSession(session.id)
      ? invoke("audio_stop", { purpose: audioPurpose(session.id) })
      : invoke("local_model_unload");
    void Promise.resolve(stopAndClose)
      .then(() => closeTerminalSession(id))
      .catch((reason: unknown) =>
        onNotify?.("error", `The server could not be stopped: ${String(reason)}`),
      );
    }, [terminalSessions, closeTerminalSession, onNotify]);

  const handleClosePanel = useCallback(() => {
    closeAllViews();
    setBrowserRequest(null);
    onUrlRequestHandled?.();
    onRequestHandled?.();
    onClose();
    void invoke("panel_browser_close").catch(() => undefined);
  }, [closeAllViews, onClose, onRequestHandled, onUrlRequestHandled]);

  /**
   * Closing the Terminal view closes its server rows too.
   *
   * The tab used to be pinned and unclosable while a process was up, which made
   * a llama-server that started but could not answer -- still a live process --
   * into a terminal the user could never dismiss. Closing the page is the one
   * gesture that says "I am finished with this", so it stops the processes first
   * and closes the view only once they have stopped. A failed stop keeps the
   * view and says why, because a row that vanished while its process survived
   * would hide the thing that is still running.
   */
  const handleCloseView = useCallback((view: PanelViewId) => {
    if (view !== "terminal") {
      closeView(view);
      return;
    }
    const servers = terminalSessions.filter((session) => session.kind === "server");
    if (servers.length === 0) {
      closeView(view);
      return;
    }
    void Promise.allSettled(servers.map((session) =>
      isAudioSession(session.id)
        ? invoke("audio_stop", { purpose: audioPurpose(session.id) })
        : invoke("local_model_unload"),
    )).then((results) => {
      const failed = results.find((result) => result.status === "rejected");
      if (failed && failed.status === "rejected") {
        onNotify?.("error", `The server could not be stopped: ${String(failed.reason)}`);
        return;
      }
      for (const session of servers) closeTerminalSession(session.id);
      closeView(view);
    });
  }, [terminalSessions, closeTerminalSession, closeView, onNotify]);

  const handleNewTerminal = useCallback(() => {
    const id = crypto.randomUUID();
    openTerminalSession({ id, kind: "user", title: `PowerShell ${terminalSessions.filter((session) => session.kind === "user").length + 1}` });
    openView("terminal");
    onOpen();
  }, [openTerminalSession, openView, onOpen, terminalSessions]);

  /**
   * Shows an HTML file in the Browser view.
   *
   * Opens the view and hands it the URL; the browser navigates on receipt, so
   * the file needs no reference to the webview itself.
   */
  const handlePreviewInBrowser = useCallback((fileUrl: string) => {
    setBrowserRequest({ url: fileUrl, nonce: Date.now() });
    openView("browser");
    onOpen();
  }, [openView, onOpen]);

  useEffect(() => {
    if (!requestedUrl || handledUrlRequestKeyRef.current === requestedUrl.key) return;
    handledUrlRequestKeyRef.current = requestedUrl.key;
    setBrowserRequest({ url: requestedUrl.url, nonce: requestedUrl.key });
    onUrlRequestHandled?.();
    openView("browser");
    onOpen();
  }, [requestedUrl, openView, onOpen, onUrlRequestHandled]);

  const handleLastBrowserTabClosed = useCallback(() => {
    setBrowserRequest(null);
    closeView("browser");
  }, [closeView]);

  /**
   * Opens the panel and brings Overview forward when a conversation is asked for.
   *
   * The last request acted on is held in a ref rather than compared against the
   * request itself: `onOpen` is an inline arrow from `App`, so it gets a new
   * identity on every render, and depending on it alone would force the panel
   * open again on each render while a conversation sat in the overview.
   */
  const handledOverviewKeyRef = useRef<number | null>(null);
  useEffect(() => {
    if (!overviewRequest || handledOverviewKeyRef.current === overviewRequest.key) return;
    handledOverviewKeyRef.current = overviewRequest.key;
    openView("overview");
    onOpen();
  }, [overviewRequest, openView, onOpen]);

  /**
   * Opens the panel and brings Files forward when a file is asked for.
   *
   * The panel does this rather than the caller, because a caller that set a path
   * and left the panel closed would look like nothing happened -- the click on a
   * changed file is a request to *see* it, and a hidden panel satisfies none of
   * it. The caller's only job is to name the file.
   */
  useEffect(() => {
    if (!requestedPath) return;
    openView("files");
    onOpen();
    // `onOpen` is a stable callback from `useRightPanel`, and `openView` too, so
    // this fires on a new request and not on every parent render. The acknowledgement
    // below is deliberately *not* a dependency: it clears the request, and
    // depending on it would re-arm this effect the moment the parent's inline
    // arrow got a new identity.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [requestedPath, openView, onOpen]);

  /**
   * Opens the panel and brings Files forward when a skill is asked for.
   *
   * The same shape as `requestedPath` and for the same reason: setting the value
   * is a request to *see* a folder, and a caller that set it and left the panel
   * closed would look like nothing happened. The last skill acted on is kept in a
   * ref so re-rendering -- `onOpen` is an inline arrow from `App` and gets a new
   * identity every render -- does not keep re-opening the view.
   */
  const handledSkillRef = useRef<string | null>(null);
  useEffect(() => {
    if (!skill) {
      handledSkillRef.current = null;
      return;
    }
    if (handledSkillRef.current === skill) return;
    handledSkillRef.current = skill;
    openView("files");
    onOpen();
  }, [skill, openView, onOpen]);

  /**
   * Moves the panel without re-rendering anything.
   *
   * The width is committed to React state only on release; during the drag this
   * writes the same `width` property directly, so the panel tracks the pointer
   * while the rest of the app -- including the transcript that reflows beside it
   * -- sits still. The browser still lays out the panel and its neighbour, but
   * it is one layout pass rather than a React render plus a reconcile of every
   * component in the window.
   */
  const applyWidth = useCallback((next: number) => {
    const panel = panelRef.current;
    if (panel) panel.style.width = `${next}px`;
  }, []);

  const handleDrag = useCallback((next: number) => {
    applyWidth(next);
  }, [applyWidth]);

  /**
   * Ends the drag: turns the transition back on, then hands the width to React.
   *
   * `transition: none` has to be removed *before* the state update, or the panel
   * would ease to the committed width on release and read as a small jump after
   * a drag that already ended where the user let go. The style is left in place
   * by React on the next render either way, so nothing has to be undone.
   */
  const handleCommit = useCallback((next: number) => {
    const panel = panelRef.current;
    if (panel) panel.style.transition = "";
    onResize(next);
  }, [onResize]);

  return (
    <aside
      ref={panelRef}
      aria-label="Right panel"
      // No `overflow-hidden` here. The body is already unmounted when closed
      // (see below), so there is nothing to clip during the width transition --
      // and clipping would also hide the open button, which is deliberately
      // positioned *outside* the zero-width box.
      //
      // The transition animates opening and closing. The handle removes it for
      // the duration of a drag -- see `ResizeHandle` -- because a 200ms ease on a
      // value changing sixty times a second makes the panel chase the cursor
      // rather than follow it, which was most of the lag on its own.
      className="relative flex min-h-0 shrink-0 border-l border-[var(--line)] bg-[var(--page)] transition-[width] duration-200"
      style={{ width: open ? width : 0 }}
    >
      {open && (
        <div className="flex min-h-0 w-full flex-col">
          {/* `min-h-[68px]` and not a fixed `h`, matching every other header in the app
              (`App.tsx`, `Sidebar.tsx`, `DatabasePage`, `StatisticsPage`). It is
              the *same* figure that makes the two dividing lines coincide: a 44px
              header put the panel's border 24px above the page's, so the row read
              as two headers at different heights rather than one band. Fixed
              height would also clip a title that wraps. */}
          <header className="flex min-h-[68px] shrink-0 items-center justify-between gap-2 border-b border-[var(--line)] bg-[var(--page)] px-5 sm:px-7">
            <PanelTabs
              openViews={openViews}
              activeView={activeView}
              onSelect={(view) => { setActiveView(view); setActiveTerminalId(null); }}
              onClose={handleCloseView}
              onNew={() => setActiveView(null)}
            />
            <button
              type="button"
              onClick={handleClosePanel}
              aria-label="Close panel"
              title="Close panel"
              className="grid size-7 shrink-0 place-items-center rounded-md text-[var(--quiet)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
            >
              <PanelRightClose size={15} />
            </button>
          </header>
          <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
            {activeView === "files" ? (
              <FilesTab workspace={workspace} skill={skill} onCloseSkill={onCloseSkill} requestedPath={requestedPath} onRequestHandled={onRequestHandled ?? (() => undefined)} onPreviewInBrowser={handlePreviewInBrowser} onNotify={onNotify} />
            ) : activeView === "terminal" ? (
              <TerminalTab
                sessions={terminalSessions}
                activeId={activeTerminalId}
                workspace={workspace}
                loadedModelId={loadedModelId}
                loadingModelId={loadingModelId}
                onSelect={setActiveTerminalId}
                onNew={handleNewTerminal}
                onClose={handleCloseTerminal}
              />
            ) : activeView === "browser" ? (
              // `open` is part of the condition because the browser's whole body is
              // unmounted when the panel closes -- that is what destroys the
              // webview, rather than hiding it and leaving its page alive.
              <BrowserTab mounted={open} initialUrl={browserRequest?.url} requestKey={browserRequest?.nonce} onRequestHandled={() => setBrowserRequest(null)} onLastTabClosed={handleLastBrowserTabClosed} onNotify={onNotify} />
            ) : activeView === "overview" && overviewRequest ? (
              // Keyed by the conversation so switching records remounts the view
              // instead of briefly drawing the previous one's figures.
              <SessionOverviewTab key={overviewRequest.id} sessionId={overviewRequest.id} loadSession={loadSession} />
            ) : activeView === "subagents" ? (
              // A link out of a run's answer opens in the panel's browser, the same
              // place a link from the chat opens -- both are the same act, so they
              // share one handler rather than growing a second way to open a page.
              <SubAgentsTab parentSessionId={sessionId} onOpenUrl={handlePreviewInBrowser} />
            ) : (
              <RightPanelEmpty onOpen={openView} hasWorkspace={Boolean(workspace)} />
            )}
          </div>
        </div>
      )}

      <ResizeHandle width={width} onDrag={handleDrag} onCommit={handleCommit} panel={panelRef} />

      {/* Closed, the whole strip is one grip you click to open.
          Deliberately not an icon: a glyph floating at the window edge is a
          control with no label and no obvious purpose, and it collided with the
          panel's own close icon in the header when open. One affordance for this
          edge, not two -- the same strip the resize handle lives on. */}
      {!open && (
        <button
          type="button"
          onClick={onOpen}
          aria-label="Open panel"
          title="Open panel"
          className="group absolute inset-y-0 right-0 z-10 w-4 cursor-col-resize focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
        >
          {/* A short centred bar, not a full-height rule.
              A hairline running the entire window height reads as a border the
              app happened to have, not as a control -- and it competes with the
              panel's own `border-l` once it opens, so the same pixel would change
              meaning between the two states. A grip centred in the middle of the
              edge reads as something to grab. The hit area stays full height so
              the click target is still easy, which is the whole reason the button
              is bigger than the mark it draws. */}
          <span className="absolute left-1/2 top-1/2 h-10 w-px -translate-x-1/2 -translate-y-1/2 rounded-full bg-[var(--line)] transition-colors group-hover:bg-[var(--accent)]" />
        </button>
      )}
    </aside>
  );
}