import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { ChartNoAxesColumn, ChartSpline, Check, ChevronLeft, ChevronRight, CircleDot, Database, MessageSquareText, MoreHorizontal, Pin, Pencil, Settings2, Trash2, X } from "lucide-react";
import type { ChatSessionHeader, SessionActivity, SessionRunStatus } from "../features/chat/types";
import { SESSION_DOT_COLORS, SESSION_DOT_DEFAULT, SESSION_DOT_DONE, SESSION_DOT_ERROR } from "../features/chat/types";

const MENU_WIDTH = 200;
const DOT_SUBMENU_WIDTH = 152;

type SidebarProps = {
  /**
   * Headers, not transcripts.
   *
   * The list draws a title, an ordering and a dot, none of which need a
   * conversation's messages -- and fetching every transcript to render this list
   * is what made startup cost grow with the size of the history.
   */
  sessions: ChatSessionHeader[];
  activeId: string | null;
  activity: SessionActivity;
  onSelectSession: (id: string) => void;
  onNewChat: () => void;
  onDeleteSession: (id: string) => void;
  onRenameSession: (id: string, title: string) => void;
  onSetDotColor: (id: string, dotColor?: string) => void;
  onTogglePinned: (id: string) => void;
  /**
   * Open the read-only overview for a conversation.
   *
   * Raised rather than handled here, because the overview needs the transcript
   * and transcripts are now fetched on demand. `App` owns that fetch and renders
   * the one modal, so the sidebar and the statistics page reach the same view
   * through the same path instead of each finding a session that may not have
   * its messages loaded.
   */
  onOpenOverview: (id: string) => void;
  onSettings: () => void;
  view: "chat" | "database" | "statistics";
  onViewChange: (view: "chat" | "database" | "statistics") => void;
};

/** One primary-nav entry. The active treatment and the collapsed layout are
    identical across Chat, Database and Statistics, so they live here rather
    than being repeated in three long class strings. */
function NavButton({
  icon,
  label,
  active,
  collapsed,
  onClick,
}: {
  icon: React.ReactNode;
  label: string;
  active: boolean;
  collapsed: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-current={active ? "page" : undefined}
      title={collapsed ? label : undefined}
      className={`flex min-h-10 w-full items-center gap-3 rounded-lg px-3 text-sm font-medium transition-colors ${collapsed ? "justify-center" : ""} ${active ? "bg-[color-mix(in_srgb,var(--accent)_13%,transparent)] text-[var(--accent)]" : "text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]"}`}
    >
      {icon}
      {!collapsed && <span>{label}</span>}
    </button>
  );
}

function LiveSpinner({ gradientId }: { gradientId: string }) {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" className="animate-spin" aria-hidden="true">
      <defs>
        <linearGradient id={gradientId} x1="0%" y1="0%" x2="100%" y2="100%">
          <stop offset="0%" stopColor="currentColor" stopOpacity="1" />
          <stop offset="60%" stopColor="currentColor" stopOpacity="0.35" />
          <stop offset="100%" stopColor="currentColor" stopOpacity="0.05" />
        </linearGradient>
      </defs>
      <circle cx="12" cy="12" r="9" fill="none" stroke={`url(#${gradientId})`} strokeWidth="3" strokeLinecap="round" strokeDasharray="42 15" />
    </svg>
  );
}

export default function Sidebar({
  sessions,
  activeId,
  activity,
  onSelectSession,
  onNewChat,
  onDeleteSession,
  onRenameSession,
  onSetDotColor,
  onTogglePinned,
  onOpenOverview,
  onSettings,
  view,
  onViewChange,
}: SidebarProps) {
  const [collapsed, setCollapsed] = useState(() => window.matchMedia("(max-width: 639px)").matches);
  const [openMenuId, setOpenMenuId] = useState<string | null>(null);
  const [dotSubmenuOpen, setDotSubmenuOpen] = useState(false);
  const dotSubmenuCloseTimer = useRef<number | null>(null);
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [menuPos, setMenuPos] = useState<{ top: number; left: number } | null>(null);
  const [menuFlipped, setMenuFlipped] = useState(false);
  // Set when the menu was opened by right-click, so it appears at the pointer
  // instead of beside the actions button.
  const [menuPoint, setMenuPoint] = useState<{ x: number; y: number } | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const dropdownRef = useRef<HTMLDivElement>(null);
  const anchorButtons = useRef(new Map<string, HTMLButtonElement>());
  const openMenuIdRef = useRef<string | null>(null);
  openMenuIdRef.current = openMenuId;

  useEffect(() => {
    const dismiss = (event: PointerEvent) => {
      const target = event.target as Node;
      const openId = openMenuIdRef.current;
      if (openId && anchorButtons.current.get(openId)?.contains(target)) return;
      if (menuRef.current && !menuRef.current.contains(target)) {
        setOpenMenuId(null);
        setDotSubmenuOpen(false);
        setMenuPoint(null);
      }
    };
    const dismissEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setOpenMenuId(null);
        setDotSubmenuOpen(false);
        setMenuPoint(null);
      }
    };
    document.addEventListener("pointerdown", dismiss);
    document.addEventListener("keydown", dismissEscape);
    return () => {
      document.removeEventListener("pointerdown", dismiss);
      document.removeEventListener("keydown", dismissEscape);
    };
  }, []);

  useLayoutEffect(() => {
    if (!openMenuId) {
      setMenuPos(null);
      return;
    }
    const place = () => {
      const height = dropdownRef.current?.offsetHeight ?? 240;
      // Right-click anchors to the pointer; the actions button anchors to itself.
      const anchor = menuPoint ? null : anchorButtons.current.get(openMenuId);
      const rect = anchor?.getBoundingClientRect();
      let left = menuPoint ? menuPoint.x : (rect?.right ?? 0) + 6;
      let top = menuPoint ? menuPoint.y : rect?.top ?? 0;
      if (left + MENU_WIDTH > window.innerWidth - 8) {
        left = Math.max(8, window.innerWidth - MENU_WIDTH - 8);
      }
      let flipped = false;
      if (top + height > window.innerHeight - 8) {
        flipped = true;
        top = Math.max(8, window.innerHeight - height - 8);
      }
      setMenuFlipped(flipped);
      setMenuPos({ top, left });
    };
    place();
    const raf = requestAnimationFrame(place);
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
    };
  }, [openMenuId, menuPoint]);

  useEffect(() => () => {
    if (dotSubmenuCloseTimer.current !== null) window.clearTimeout(dotSubmenuCloseTimer.current);
  }, []);

  const openDotSubmenu = () => {
    if (dotSubmenuCloseTimer.current !== null) {
      window.clearTimeout(dotSubmenuCloseTimer.current);
      dotSubmenuCloseTimer.current = null;
    }
    setDotSubmenuOpen(true);
  };

  const scheduleDotSubmenuClose = () => {
    if (dotSubmenuCloseTimer.current !== null) window.clearTimeout(dotSubmenuCloseTimer.current);
    dotSubmenuCloseTimer.current = window.setTimeout(() => setDotSubmenuOpen(false), 140);
  };

  const beginRename = (session: ChatSessionHeader) => {
    setRenameValue(session.title);
    setRenamingId(session.id);
    setOpenMenuId(null);
    setDotSubmenuOpen(false);
    setMenuPoint(null);
  };

  const closeMenu = () => {
    setOpenMenuId(null);
    setDotSubmenuOpen(false);
    setMenuPoint(null);
  };

  const toggleMenu = (id: string) => {
    setMenuPoint(null);
    setOpenMenuId((current) => {
      const next = current === id ? null : id;
      if (next === null) setDotSubmenuOpen(false);
      return next;
    });
  };

  // Right-clicking a session opens the same menu at the pointer, which is the
  // expected gesture and saves reaching for the actions button.
  const openMenuAtPointer = (event: React.MouseEvent, session: ChatSessionHeader) => {
    if (renamingId === session.id) return;
    event.preventDefault();
    setDotSubmenuOpen(false);
    setMenuPoint({ x: event.clientX, y: event.clientY });
    setOpenMenuId(session.id);
  };

  const finishRename = (id: string) => {
    onRenameSession(id, renameValue);
    setRenamingId(null);
  };

  // Portalled so the fixed overlay is not clipped by the sidebar's layout.
  const sidebar = (
    <aside className={`flex h-full shrink-0 flex-col border-r border-[var(--line)] bg-[var(--rail)] transition-[width] duration-200 ${collapsed ? "w-[68px]" : "w-[260px] max-sm:fixed max-sm:inset-y-0 max-sm:left-0 max-sm:z-50 max-sm:shadow-2xl"}`}>
      <div className={`relative flex h-[68px] items-center gap-3 border-b border-[var(--line)] ${collapsed ? "justify-center px-2" : "px-4"}`}>
        {!collapsed && <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-semibold tracking-tight">ProjectZ</p>
          <p className="text-xs text-[var(--quiet)]">AI workspace</p>
        </div>}
        <button
          type="button"
          className={`grid size-9 shrink-0 place-items-center rounded-lg text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)] ${collapsed ? "absolute right-1 top-1/2 -translate-y-1/2" : "grid"}`}
          onClick={() => setCollapsed((value) => !value)}
          aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
          title={collapsed ? "Expand sidebar" : "Collapse sidebar"}
        >
          {collapsed ? <ChevronRight size={17} /> : <ChevronLeft size={17} />}
        </button>
      </div>

      <nav aria-label="Primary" className="space-y-1 px-3 py-4">
        <NavButton
          icon={<MessageSquareText size={17} />}
          label="Chat"
          active={view === "chat"}
          collapsed={collapsed}
          onClick={() => { onViewChange("chat"); onNewChat(); }}
        />
        <NavButton
          icon={<Database size={17} />}
          label="Database"
          active={view === "database"}
          collapsed={collapsed}
          onClick={() => onViewChange("database")}
        />
        <NavButton
          icon={<ChartSpline size={17} />}
          label="Statistics"
          active={view === "statistics"}
          collapsed={collapsed}
          onClick={() => onViewChange("statistics")}
        />
      </nav>

      {!collapsed && <section aria-label="Conversation history" className="flex min-h-0 flex-1 flex-col px-3 pb-3">
        <div className="flex h-10 items-center px-2">
          <h2 className="text-xs font-medium text-[var(--muted)]">History</h2>
        </div>
        <div className="min-h-0 flex-1 space-y-1 overflow-y-auto">
          {sessions.length === 0 && <p className="px-2 py-3 text-xs text-[var(--quiet)]">Your conversations will appear here.</p>}
          {sessions.map((session) => {
            const status: SessionRunStatus | undefined = activity[session.id];
            const isActive = session.id === activeId;
            const isLive = status === "streaming" || status === "searching";
            const menuOpen = openMenuId === session.id;
            const flipsLeft = menuPos !== null && menuPos.left + MENU_WIDTH + DOT_SUBMENU_WIDTH + 12 > window.innerWidth - 8;
            const defaultDot = session.dotColor || SESSION_DOT_DEFAULT;
            const dot = isLive ? "var(--accent)" : status === "done" ? SESSION_DOT_DONE : status === "error" ? SESSION_DOT_ERROR : defaultDot;
            return <div key={session.id} onContextMenu={(event) => openMenuAtPointer(event, session)} className={`group flex items-center gap-1 rounded-lg pr-1 ${isActive ? "bg-[var(--raised)] text-[var(--text)]" : "text-[var(--muted)] hover:bg-[color-mix(in_srgb,var(--raised)_65%,transparent)] hover:text-[var(--text)]"}`}>
              {renamingId === session.id ? <form onSubmit={(event) => { event.preventDefault(); finishRename(session.id); }} className="flex min-h-10 min-w-0 flex-1 items-center gap-1 px-1">
                <input autoFocus aria-label="Session name" value={renameValue} onChange={(event) => setRenameValue(event.target.value)} onKeyDown={(event) => { if (event.key === "Escape") setRenamingId(null); }} className="min-w-0 flex-1 rounded bg-[var(--page)] px-2 py-1 text-[13px] text-[var(--text)] outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)]" />
                <button type="submit" className="grid size-7 place-items-center rounded hover:bg-[var(--page)]" aria-label="Save session name"><Check size={14} /></button>
                <button type="button" onClick={() => setRenamingId(null)} className="grid size-7 place-items-center rounded hover:bg-[var(--page)]" aria-label="Cancel rename"><X size={14} /></button>
              </form> : <button type="button" onClick={() => { onSelectSession(session.id); onViewChange("chat"); }} className="flex min-h-10 min-w-0 flex-1 items-center gap-2.5 px-2.5 text-left text-[13px]" title={session.title}>
                {session.pinned ? <Pin size={14} className="shrink-0 text-[var(--accent)]" /> : <span aria-hidden="true" title={status === "searching" ? "Searching the web" : isLive ? "Streaming response" : status === "done" ? "Response ready" : status === "error" ? "Response failed" : "Conversation"} className="grid size-4 shrink-0 place-items-center"><span className="inline-flex size-1.5 rounded-full" style={{ backgroundColor: dot }} /></span>}
                <span className="truncate">{session.title}</span>
              </button>}
              <div className="relative shrink-0">
                {isLive && !menuOpen ? <>
                  <span role="status" aria-label="AI is writing" title="AI is writing" className="grid size-8 place-items-center text-[var(--muted)] group-hover:hidden group-focus-within:hidden"><LiveSpinner gradientId={`pz-spin-${session.id}`} /></span>
                  <button ref={(el) => { if (el) anchorButtons.current.set(session.id, el); else anchorButtons.current.delete(session.id); }} type="button" onClick={() => toggleMenu(session.id)} aria-label={`Actions for ${session.title}`} aria-expanded={false} title="Session actions" className="hidden size-8 place-items-center rounded-md text-[var(--quiet)] hover:bg-[var(--page)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-[var(--accent)] group-hover:grid group-focus-within:grid">
                    <MoreHorizontal size={16} />
                  </button>
                </> : <button ref={(el) => { if (el) anchorButtons.current.set(session.id, el); else anchorButtons.current.delete(session.id); }} type="button" onClick={() => toggleMenu(session.id)} aria-label={`Actions for ${session.title}`} aria-expanded={menuOpen} title="Session actions" className={`grid size-8 place-items-center rounded-md text-[var(--quiet)] hover:bg-[var(--page)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-[var(--accent)] ${menuOpen ? "opacity-100" : "opacity-0 group-hover:opacity-100 group-focus-within:opacity-100"}`}>
                  <MoreHorizontal size={16} />
                </button>}
                {menuOpen && menuPos && createPortal(<div ref={(el) => { menuRef.current = el; dropdownRef.current = el; }} aria-label={`${session.title} actions`} style={{ position: "fixed", top: menuPos.top, left: menuPos.left, width: MENU_WIDTH }} className="z-[100] overflow-visible rounded-lg border border-[var(--line)] bg-[var(--rail)] p-1 shadow-2xl">
                  <button type="button" onClick={() => { onOpenOverview(session.id); closeMenu(); }} className="flex min-h-8 w-full items-center gap-2.5 rounded-md px-2 text-left text-xs text-[var(--text)] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)]"><ChartNoAxesColumn size={14} className="text-[var(--muted)]" />Overview</button>
                  <div className="my-1 border-t border-[var(--line)]" />
                  <button type="button" onClick={() => { onTogglePinned(session.id); closeMenu(); }} className="flex min-h-8 w-full items-center gap-2.5 rounded-md px-2 text-left text-xs text-[var(--text)] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)]"><Pin size={14} className="text-[var(--muted)]" />{session.pinned ? "Unpin" : "Pin"}</button>
                  <button type="button" onClick={() => beginRename(session)} className="flex min-h-8 w-full items-center gap-2.5 rounded-md px-2 text-left text-xs text-[var(--text)] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)]"><Pencil size={14} className="text-[var(--muted)]" />Rename</button>
                  <div className="relative" onMouseEnter={openDotSubmenu} onMouseLeave={scheduleDotSubmenuClose}>
                    <button type="button" aria-haspopup="menu" aria-expanded={dotSubmenuOpen} onClick={() => setDotSubmenuOpen((open) => !open)} onFocus={openDotSubmenu} className="flex min-h-8 w-full items-center gap-2.5 rounded-md px-2 text-left text-xs text-[var(--text)] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)]"><CircleDot size={14} className="text-[var(--muted)]" />Dot style<ChevronRight size={13} className="ml-auto text-[var(--quiet)]" /></button>
                    {dotSubmenuOpen && <div role="menu" aria-label="Session dot color" onMouseEnter={openDotSubmenu} onMouseLeave={scheduleDotSubmenuClose} style={{ width: DOT_SUBMENU_WIDTH }} className={`absolute top-0 z-[100] rounded-lg border border-[var(--line)] bg-[var(--rail)] p-2 shadow-2xl ${flipsLeft ? "right-full mr-1" : "left-full ml-1"} ${menuFlipped ? "bottom-0 top-auto" : ""}`}>
                      <div className="grid grid-cols-4 gap-2">
                        {SESSION_DOT_COLORS.map((color) => <button key={color} type="button" role="menuitemradio" aria-checked={defaultDot === color} aria-label={`Dot color ${color}`} title={color} onClick={() => { onSetDotColor(session.id, color === SESSION_DOT_DEFAULT ? undefined : color); closeMenu(); }} className={`grid size-7 shrink-0 place-items-center rounded-md hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)] ${defaultDot === color ? "bg-[var(--raised)] ring-1 ring-[var(--accent)]" : ""}`}><span className="size-3.5 shrink-0 rounded-full" style={{ backgroundColor: color }} /></button>)}
                      </div>
                    </div>}
                  </div>
                  <div className="my-1 border-t border-[var(--line)]" />
                  <button type="button" onClick={() => { onDeleteSession(session.id); closeMenu(); }} className="flex min-h-8 w-full items-center gap-2.5 rounded-md px-2 text-left text-xs text-[var(--danger)] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)]"><Trash2 size={14} />Delete</button>
                </div>, document.body)}
              </div>
            </div>;
          })}
        </div>
      </section>}

      <div className="mt-auto border-t border-[var(--line)] p-3">
        <button type="button" onClick={onSettings} className={`flex min-h-10 w-full items-center gap-3 rounded-lg px-3 text-sm text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)] ${collapsed ? "justify-center" : ""}`} title={collapsed ? "Settings" : undefined}>
          <Settings2 size={17} />
          {!collapsed && <span>Settings</span>}
        </button>
      </div>
    </aside>
  );

  // Portalled so the fixed overlay is not clipped by the sidebar's layout.
  return (
    <>
      {sidebar}
      {/* The overview modal is rendered by `App`, which owns the transcript fetch.
          Kept out of here so the sidebar and the statistics page open the same
          view through one path rather than each resolving a session itself. */}
    </>
  );
}
