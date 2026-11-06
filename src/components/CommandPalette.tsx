import { Fragment, useEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { FileText, MessageSquareText, Search, Settings2 } from "lucide-react";
import type { ChatSessionHeader } from "../features/chat/types";
import { buildSettingsIndex, type SettingsEntry } from "./settings/settingsIndex";

/**
 * One search box for the things a user reaches for by name.
 *
 * Conversations, settings rows and workspace files already exist as three
 * separate surfaces with three separate ways in -- the sidebar, the settings
 * search, and the Files tree. Ctrl+K is the one gesture that does not make the
 * user remember which of them owns the thing they are thinking of.
 *
 * Files are searched on the backend through `panel_search_paths`, the same walk
 * the Files panel uses: the palette cannot see which folders are expanded, and
 * a search over the open tree would miss the file nobody had opened.
 */

type CommandPaletteProps = {
  sessions: ChatSessionHeader[];
  onSelectSession: (id: string) => void;
  onOpenSettingsEntry: (entry: SettingsEntry) => void;
  onOpenFile: (path: string) => void;
  onClose: () => void;
};

type Row = {
  key: string;
  label: string;
  hint: string;
  group: "Conversations" | "Settings" | "Files";
  run: () => void;
};

const GROUP_ICONS = {
  Conversations: MessageSquareText,
  Settings: Settings2,
  Files: FileText,
} as const;

export default function CommandPalette({
  sessions,
  onSelectSession,
  onOpenSettingsEntry,
  onOpenFile,
  onClose,
}: CommandPaletteProps) {
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const [files, setFiles] = useState<{ name: string; path: string }[]>([]);
  const [searchingFiles, setSearchingFiles] = useState(false);
  const inputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  // The same debounce the Files panel uses: a keystroke does not start a walk.
  useEffect(() => {
    const needle = query.trim();
    if (!needle) {
      setFiles([]);
      setSearchingFiles(false);
      return;
    }
    let cancelled = false;
    setSearchingFiles(true);
    const timer = window.setTimeout(() => {
      invoke<{ name: string; path: string }[]>("panel_search_paths", { query: needle, skill: null })
        .then((hits) => {
          if (cancelled) return;
          setFiles(hits.slice(0, 8));
          setSearchingFiles(false);
        })
        .catch(() => {
          if (cancelled) return;
          setFiles([]);
          setSearchingFiles(false);
        });
    }, 150);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [query]);

  const settings = useMemo(() => buildSettingsIndex(), []);

  const rows = useMemo<Row[]>(() => {
    const needle = query.trim().toLowerCase();
    const match = (text: string) => !needle || text.toLowerCase().includes(needle);

    const conversations = sessions
      .filter((session) => match(session.title))
      .slice(0, needle ? 8 : 6)
      .map<Row>((session) => ({
        key: `session:${session.id}`,
        label: session.title,
        hint: "Conversation",
        group: "Conversations",
        run: () => onSelectSession(session.id),
      }));

    // With no query the list is recent conversations only: a hundred settings
    // rows with nothing asked is a menu, not a search.
    const settingRows = needle
      ? settings
          .filter((entry) => match(`${entry.label} ${entry.page} ${entry.keywords.join(" ")}`))
          .slice(0, 8)
          .map<Row>((entry) => ({
            key: `setting:${entry.anchor}`,
            label: entry.label,
            hint: entry.page,
            group: "Settings",
            run: () => onOpenSettingsEntry(entry),
          }))
      : [];

    const fileRows = files.map<Row>((file) => ({
      key: `file:${file.path}`,
      label: file.name,
      hint: file.path,
      group: "Files",
      run: () => onOpenFile(file.path),
    }));

    return [...conversations, ...settingRows, ...fileRows];
  }, [sessions, settings, files, query, onSelectSession, onOpenSettingsEntry, onOpenFile]);

  // A new query is a new list, and the old highlight would point at whatever now
  // sits at that index.
  useEffect(() => {
    setActive(0);
  }, [query]);

  const choose = (row: Row | undefined) => {
    if (!row) return;
    onClose();
    row.run();
  };

  const onKeyDown = (event: ReactKeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      onClose();
    } else if (event.key === "ArrowDown") {
      event.preventDefault();
      setActive((index) => Math.min(index + 1, Math.max(0, rows.length - 1)));
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      setActive((index) => Math.max(index - 1, 0));
    } else if (event.key === "Enter") {
      event.preventDefault();
      choose(rows[active]);
    }
  };

  return (
    <div
      className="fixed inset-0 z-[60] flex items-start justify-center bg-black/40 pt-[12vh]"
      onMouseDown={onClose}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Command palette"
        className="w-[min(560px,calc(100vw-2rem))] overflow-hidden rounded-xl border border-[var(--line)] bg-[var(--panel)] shadow-2xl"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <div className="flex items-center gap-2 border-b border-[var(--line)] px-3 py-2.5">
          <Search size={14} className="shrink-0 text-[var(--quiet)]" aria-hidden />
          <input
            ref={inputRef}
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={onKeyDown}
            placeholder="Search conversations, settings and files"
            aria-label="Search conversations, settings and files"
            className="min-w-0 flex-1 bg-transparent text-[13px] text-[var(--text)] outline-none placeholder:text-[var(--quiet)]"
          />
          <kbd className="shrink-0 rounded border border-[var(--line)] px-1.5 py-0.5 text-[10px] text-[var(--quiet)]">Esc</kbd>
        </div>
        <div className="max-h-[50vh] overflow-y-auto py-1">
          {rows.length === 0 && (
            <p className="px-3 py-6 text-center text-[12px] text-[var(--quiet)]">
              {searchingFiles ? "Searching files…" : query.trim() ? "Nothing matches." : "No conversations yet."}
            </p>
          )}
          {rows.map((row, index) => {
            const Icon = GROUP_ICONS[row.group];
            return (
              <Fragment key={row.key}>
                {(index === 0 || rows[index - 1].group !== row.group) && (
                  <p className="px-3 pb-1 pt-2 text-[10px] uppercase tracking-wide text-[var(--quiet)]">{row.group}</p>
                )}
                <button
                  type="button"
                  onMouseDown={() => choose(row)}
                  onMouseEnter={() => setActive(index)}
                  className={`flex w-full items-center gap-2 px-3 py-1.5 text-left text-[13px] ${index === active ? "bg-[var(--raised)] text-[var(--text)]" : "text-[var(--muted)]"}`}
                >
                  <Icon size={13} className="shrink-0 text-[var(--quiet)]" aria-hidden />
                  <span className="min-w-0 flex-1 truncate">{row.label}</span>
                  <span className="max-w-[45%] shrink-0 truncate text-[11px] text-[var(--quiet)]">{row.hint}</span>
                </button>
              </Fragment>
            );
          })}
        </div>
      </div>
    </div>
  );
}
