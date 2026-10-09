import { Fragment, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { ChartNoAxesColumn, ChartSpline, Check, ChevronLeft, ChevronRight, CircleDot, Clock, Database, MessageSquareText, MoreHorizontal, Pin, Pencil, Settings2, Trash2, X } from "lucide-react";
import type { ChatSessionHeader, SessionActivity, SessionRunStatus } from "../features/chat/types";
import { SESSION_DOT_COLORS, SESSION_DOT_DEFAULT, SESSION_DOT_DONE, SESSION_DOT_ERROR } from "../features/chat/types";
import LiveSpinner from "./LiveSpinner";

const MENU_WIDTH = 200;
const DOT_SUBMENU_WIDTH = 152;

/**
 * The recency heading a conversation sits under in the sidebar.
 *
 * Recency rather than alphabetical or by model, because the question the list
 * answers is "the one I was just in" far more often than any other. Pinned
 * conversations are their own heading regardless of age, which is the whole
 * point of pinning.
 *
 * Days are compared on local calendar boundaries rather than a rolling 24
 * hours: something from 11pm last night reads as "Yesterday" to a person, and
 * a list that filed it under "Today" until 11pm the next evening would be
 * disagreeing with the reader about what day it is.
 */
function sessionGroupLabel(session: ChatSessionHeader): string {
  // Jobs first, and before the pin: a job run belongs in its own section whatever
  // else is true of it, and the list is ordered so the section is contiguous.
  if (session.kind === "job") return "Jobs";
  if (session.pinned) return "Pinned";
  const updated = new Date(session.updatedAt);
  if (Number.isNaN(updated.getTime())) return "Earlier";
  const startOfToday = new Date();
  startOfToday.setHours(0, 0, 0, 0);
  const dayStart = startOfToday.getTime();
  const time = updated.getTime();
  if (time >= dayStart) return "Today";
  if (time >= dayStart - 86_400_000) return "Yesterday";
  if (time >= dayStart - 7 * 86_400_000) return "Previous 7 days";
  return "Earlier";
}

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
   * and transcripts are fetched on demand. `App` raises the request and the right
   * panel renders the one view, so the sidebar and the statistics page reach the
   * same view through the same path instead of each finding a session that may
   * not have its messages loaded.
   */
  onOpenOverview: (id: string) => void;
  onSettings: () => void;
  view: "chat" | "database" | "statistics" | "jobs";
  onViewChange: (view: "chat" | "database" | "statistics" | "jobs") => void;
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
      className={`flex min-h-9 w-full items-center gap-3 rounded-md px-3 text-[13px] font-medium transition-colors ${collapsed ? "justify-center" : ""} ${active ? "bg-[color-mix(in_srgb,var(--accent)_13%,transparent)] text-[var(--accent)]" : "text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]"}`}
    >
      {icon}
      {!collapsed && <span>{label}</span>}
    </button>
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
      <div className={`flex h-[60px] shrink-0 items-center gap-2 border-b border-[var(--line)] ${collapsed ? "justify-center px-2" : "px-4"}`}>
        {!collapsed && <p className="min-w-0 flex-1 truncate text-[13px] font-semibold tracking-tight">ProjectZ</p>}
        <button
          type="button"
          className="grid size-7 shrink-0 place-items-center rounded-md text-[var(--quiet)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)]"
          onClick={() => setCollapsed((value) => !value)}
          aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
          title={collapsed ? "Expand sidebar" : "Collapse sidebar"}
        >
          {collapsed ? <ChevronRight size={16} /> : <ChevronLeft size={16} />}
        </button>
      </div>

      <nav aria-label="Primary" className="space-y-0.5 px-3 pt-3">
        <NavButton
          icon={<MessageSquareText size={16} />}
          label="Chat"
          active={view === "chat"}
          collapsed={collapsed}
          onClick={() => { onViewChange("chat"); onNewChat(); }}
        />
        <NavButton
          icon={<Database size={16} />}
          label="Database"
          active={view === "database"}
          collapsed={collapsed}
          onClick={() => onViewChange("database")}
        />
        <NavButton
          icon={<ChartSpline size={16} />}
          label="Statistics"
          active={view === "statistics"}
          collapsed={collapsed}
          onClick={() => onViewChange("statistics")}
        />
        <NavButton
          icon={<Clock size={16} />}
          label="Jobs"
          active={view === "jobs"}
          collapsed={collapsed}
          onClick={() => onViewChange("jobs")}
        />
      </nav>

      {!collapsed && <section aria-label="Conversation history" className="mt-3 flex min-h-0 flex-1 flex-col border-t border-[var(--line)] px-3 pb-3">
        {/* No "History" heading above the groups. Each group names itself, and a
            heading over a list of headings is a label doing no work. */}
        <div className="min-h-0 flex-1 space-y-0.5 overflow-y-auto pt-1">
          {sessions.length === 0 && <p className="px-2 py-3 text-xs leading-5 text-[var(--quiet)]">Nothing here yet. Your conversations collect here once you start one.</p>}
          {sessions.map((session, index) => {
            // A heading is emitted whenever the bucket changes, rather than the
            // list being pre-bucketed: the array is already ordered pinned-first
            // then most-recent, so a change of label is exactly where a heading
            // belongs and nothing has to be sorted twice.
            const groupLabel = sessionGroupLabel(session);
            const showGroup = index === 0 || sessionGroupLabel(sessions[index - 1]) !== groupLabel;
            const status: SessionRunStatus | undefined = activity[session.id];
            const isActive = session.id === activeId;
            const isLive = status === "streaming" || status === "searching";
            const menuOpen = openMenuId === session.id;
            const flipsLeft = menuPos !== null && menuPos.left + MENU_WIDTH + DOT_SUBMENU_WIDTH + 12 > window.innerWidth - 8;
            const defaultDot = session.dotColor || SESSION_DOT_DEFAULT;
            const dot = isLive ? "var(--accent)" : status === "done" ? SESSION_DOT_DONE : status === "error" ? SESSION_DOT_ERROR : defaultDot;
            return <Fragment key={session.id}>
              {showGroup && <h3 className="px-2 pb-1 pt-3 text-[11px] font-medium text-[var(--quiet)]">{groupLabel}</h3>}
              <div onContextMenu={(event) => openMenuAtPointer(event, session)} className={`group flex items-center gap-1 rounded-md pr-1 ${isActive ? "bg-[var(--raised)] text-[var(--text)]" : "text-[var(--muted)] hover:bg-[color-mix(in_srgb,var(--raised)_65%,transparent)] hover:text-[var(--text)]"}`}>
              {renamingId === session.id ? <form onSubmit={(event) => { event.preventDefault(); finishRename(session.id); }} className="flex min-h-9 min-w-0 flex-1 items-center gap-1 px-1">
                <input autoFocus aria-label="Session name" value={renameValue} onChange={(event) => setRenameValue(event.target.value)} onKeyDown={(event) => { if (event.key === "Escape") setRenamingId(null); }} className="min-w-0 flex-1 rounded bg-[var(--page)] px-2 py-1 text-[13px] text-[var(--text)] outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)]" />
                <button type="submit" className="grid size-7 place-items-center rounded hover:bg-[var(--page)]" aria-label="Save session name"><Check size={14} /></button>
                <button type="button" onClick={() => setRenamingId(null)} className="grid size-7 place-items-center rounded hover:bg-[var(--page)]" aria-label="Cancel rename"><X size={14} /></button>
              </form> : <button type="button" onClick={() => { onSelectSession(session.id); onViewChange("chat"); }} className="flex min-h-9 min-w-0 flex-1 items-center gap-2.5 px-2.5 text-left text-[13px]" title={session.title}>
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
              </div>
            </Fragment>;
          })}
        </div>
      </section>}

      <div className="mt-auto border-t border-[var(--line)] p-3">
        <button type="button" onClick={onSettings} className={`flex min-h-9 w-full items-center gap-3 rounded-md px-3 text-[13px] text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)] ${collapsed ? "justify-center" : ""}`} title={collapsed ? "Settings" : undefined}>
          <Settings2 size={16} />
          {!collapsed && <span>Settings</span>}
        </button>
      </div>
    </aside>
  );

  // Portalled so the fixed overlay is not clipped by the sidebar's layout.
  return (
    <>
      {sidebar}
      {/* The overview is rendered by the right panel, which the request opens.
          Kept out of here so the sidebar and the statistics page open the same
          view through one path rather than each resolving a session itself. */}
    </>
  );
}
