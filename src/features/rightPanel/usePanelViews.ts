import { useCallback, useEffect, useRef, useState } from "react";
import { PANEL_VIEWS_KEY, readPanelSetting, writePanelSetting } from "./panelMemory";

export type PanelViewId = "files" | "browser" | "terminal" | "subagents" | "overview" | "taste";
export type TerminalSession = { id: string; kind: "server" | "user"; title: string };

/**
 * Views that can be put back on the next launch.
 *
 * `overview` and `taste` are requests, not places: both are opened with something
 * to look at (a conversation id, the latest run), and restoring either would draw
 * a view with nothing behind it. `terminal` is tied to live processes and has no
 * rows to come back to. Files, the browser and the sub-agent list stand on their
 * own, so those are the three the panel remembers.
 */
const RESTORABLE: PanelViewId[] = ["files", "browser", "subagents"];

export function usePanelViews() {
  const [openViews, setOpenViews] = useState<PanelViewId[]>([]);
  const [activeView, setActiveViewState] = useState<PanelViewId | null>(null);
  const [terminalSessions, setTerminalSessions] = useState<TerminalSession[]>([]);
  const [activeTerminalId, setActiveTerminalId] = useState<string | null>(null);

  /**
   * Whether the saved views have been read.
   *
   * Writes wait for this, so the first empty render cannot overwrite the record
   * before it has been loaded -- the same rule `useRightPanel` follows for the
   * width.
   */
  const loadedRef = useRef(false);
  /** True once the user has opened or closed something themselves. */
  const interactedRef = useRef(false);

  useEffect(() => {
    let live = true;
    void readPanelSetting<{ views?: string[]; active?: string | null }>(PANEL_VIEWS_KEY).then((saved) => {
      if (!live) return;
      // A view opened while the read was in flight is newer than the record.
      if (!saved || interactedRef.current) {
        loadedRef.current = true;
        return;
      }
      const views = (saved.views ?? []).filter(
        (view): view is PanelViewId => RESTORABLE.includes(view as PanelViewId),
      );
      if (views.length > 0) {
        setOpenViews(views);
        const remembered = views.includes(saved.active as PanelViewId)
          ? (saved.active as PanelViewId)
          : views[views.length - 1];
        setActiveViewState(remembered);
      }
      loadedRef.current = true;
    });
    return () => { live = false; };
  }, []);

  useEffect(() => {
    if (!loadedRef.current) return;
    writePanelSetting(PANEL_VIEWS_KEY, { views: openViews, active: activeView });
  }, [openViews, activeView]);

  const open = useCallback((view: PanelViewId) => {
    interactedRef.current = true;
    setOpenViews((current) => (current.includes(view) ? current : [...current, view]));
    setActiveViewState(view);
  }, []);

  const close = useCallback((view: PanelViewId) => {
    interactedRef.current = true;
    const remaining = openViews.filter((entry) => entry !== view);
    setOpenViews(remaining);
    if (view === activeView) setActiveViewState(remaining[remaining.length - 1] ?? null);
  }, [activeView, openViews]);

  /** The same setter the panel already uses, marked as a deliberate choice. */
  const setActiveView = useCallback((view: PanelViewId | null) => {
    interactedRef.current = true;
    setActiveViewState(view);
  }, []);

  const openTerminalSession = useCallback((session: TerminalSession) => {
    setTerminalSessions((current) => current.some((entry) => entry.id === session.id) ? current : [...current, session]);
    setActiveTerminalId(session.id);
  }, []);

  const closeTerminalSession = useCallback((id: string) => {
    const remaining = terminalSessions.filter((entry) => entry.id !== id);
    setTerminalSessions(remaining);
    if (activeTerminalId === id) setActiveTerminalId(remaining[remaining.length - 1]?.id ?? null);
  }, [activeTerminalId, terminalSessions]);

  const ensureServerSession = useCallback((id: string, title: string) => {
    setTerminalSessions((current) => current.some((entry) => entry.id === id)
      ? current.map((entry) => entry.id === id ? { ...entry, title } : entry)
      : [...current, { id, kind: "server", title }]);
    setActiveTerminalId(id);
  }, []);

  return {
    openViews,
    activeView,
    open,
    close,
    setActiveView,
    terminalSessions,
    activeTerminalId,
    setActiveTerminalId,
    openTerminalSession,
    closeTerminalSession,
    ensureServerSession,
  };
}
