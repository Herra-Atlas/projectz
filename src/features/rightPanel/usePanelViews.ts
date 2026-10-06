import { useCallback, useState } from "react";

export type PanelViewId = "files" | "browser" | "terminal";
export type TerminalSession = { id: string; kind: "server" | "user"; title: string };

export function usePanelViews() {
  const [openViews, setOpenViews] = useState<PanelViewId[]>([]);
  const [activeView, setActiveView] = useState<PanelViewId | null>(null);
  const [terminalSessions, setTerminalSessions] = useState<TerminalSession[]>([]);
  const [activeTerminalId, setActiveTerminalId] = useState<string | null>(null);

  const open = useCallback((view: PanelViewId) => {
    setOpenViews((current) => (current.includes(view) ? current : [...current, view]));
    setActiveView(view);
  }, []);

  const close = useCallback((view: PanelViewId) => {
    const remaining = openViews.filter((entry) => entry !== view);
    setOpenViews(remaining);
    if (view === activeView) setActiveView(remaining[remaining.length - 1] ?? null);
  }, [activeView, openViews]);

  const openTerminalSession = useCallback((session: TerminalSession) => {
    setTerminalSessions((current) => current.some((entry) => entry.id === session.id) ? current : [...current, session]);
    setActiveTerminalId(session.id);
  }, []);

  const closeTerminalSession = useCallback((id: string, serverRunning: boolean) => {
    const session = terminalSessions.find((entry) => entry.id === id);
    if (!session || (session.kind === "server" && serverRunning)) return;
    const remaining = terminalSessions.filter((entry) => entry.id !== id);
    setTerminalSessions(remaining);
    if (activeTerminalId === id) setActiveTerminalId(remaining[remaining.length - 1]?.id ?? null);
  }, [activeTerminalId, terminalSessions]);

  const closeAll = useCallback((serverRunning: boolean) => {
    setOpenViews([]);
    setActiveView(null);
    const pinned = terminalSessions.filter((entry) => entry.kind === "server" && serverRunning);
    setTerminalSessions(pinned);
    setActiveTerminalId(pinned[0]?.id ?? null);
  }, [terminalSessions]);

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
    closeAll,
    ensureServerSession,
  };
}
