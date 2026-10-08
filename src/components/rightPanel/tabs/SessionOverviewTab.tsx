import { useEffect, useRef, useState } from "react";
import { LoaderCircle } from "lucide-react";
import { computeSessionStats, formatTimestamp } from "../../../features/chat/sessionStats";
import type { ChatSession } from "../../../features/chat/types";
import TabStrip, { type SessionOverviewTab as SessionOverviewTabName } from "../../session-overview/TabStrip";
import OverviewTab from "../../session-overview/tabs/OverviewTab";
import StatisticsTab from "../../session-overview/tabs/StatisticsTab";
import AdvancedTab from "../../session-overview/tabs/AdvancedTab";

type SessionOverviewTabProps = {
  /** The conversation whose record is shown. */
  sessionId: string;
  /**
   * Reads one conversation with its transcript.
   *
   * Passed in rather than imported so the panel shares the chat hook's cache:
   * a conversation already opened in the chat view is not read again to show
   * these figures, and the two views can never disagree about its messages.
   */
  loadSession: (id: string) => Promise<ChatSession | null>;
};

/** A conversation's read-only record, as a right-panel view.
 *
 * The same figures the overview always showed, with the same `session-overview/`
 * pieces beneath it -- only the frame changes. The transcript is read here, on
 * demand, so clicking Overview never opens the conversation itself: the chat
 * view keeps whatever it was showing.
 *
 * It returns a full-height column: a title band, the tab strip, and a scrolling
 * body. That is the shape every panel view takes, so the panel's own tab strip
 * sits above it exactly as it does for Files or the sub agents.
 */
export default function SessionOverviewTab({ sessionId, loadSession }: SessionOverviewTabProps) {
  const [session, setSession] = useState<ChatSession | null>(null);
  const [error, setError] = useState("");
  const [tab, setTab] = useState<SessionOverviewTabName>("Overview");

  // The loader closes over the chat hook's transcript cache, so its identity
  // changes as transcripts arrive. Read through a ref so a cache update does
  // not re-run the effect below and reload the conversation being shown.
  const loadRef = useRef(loadSession);
  loadRef.current = loadSession;

  useEffect(() => {
    let cancelled = false;
    // Cleared first: drawing one conversation's figures under another's title
    // while the new transcript is in flight would report the wrong record.
    setSession(null);
    setError("");
    // A new conversation opens on the leading tab rather than inheriting the
    // one the last record was read on, since its figures are not comparable.
    setTab("Overview");
    loadRef.current(sessionId).then((loaded) => {
      if (cancelled) return;
      if (loaded) setSession(loaded);
      else setError("This conversation could not be read.");
    }).catch((reason: unknown) => {
      if (!cancelled) setError(String(reason));
    });
    return () => { cancelled = true; };
  }, [sessionId]);

  if (error) {
    return (
      <div className="flex h-full flex-col items-center justify-center px-5 py-8 text-center">
        <p className="text-sm text-[var(--muted)]">{error}</p>
      </div>
    );
  }

  if (!session) {
    return (
      <div className="flex h-full items-center justify-center gap-2 px-5 py-8 text-sm text-[var(--muted)]">
        <LoaderCircle size={15} className="animate-spin text-[var(--accent)]" />
        Reading the conversation…
      </div>
    );
  }

  const stats = computeSessionStats(session);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {/* The conversation's name and span. The panel strip above already names
          the view, so this names the record instead of repeating "Overview". */}
      <header className="shrink-0 border-b border-[var(--line)] px-5 py-3">
        <h2 className="truncate text-sm font-semibold text-[var(--text)]" title={session.title}>{session.title}</h2>
        <p className="mt-0.5 truncate text-[11px] text-[var(--quiet)]">
          {formatTimestamp(stats.createdAt)} &rarr; {formatTimestamp(stats.updatedAt)}
        </p>
      </header>

      <TabStrip value={tab} onChange={setTab} />

      <div
        id="session-tab-panel"
        role="tabpanel"
        aria-labelledby={`session-tab-${tab.toLowerCase()}`}
        className="min-h-0 flex-1 overflow-y-auto"
      >
        {tab === "Overview" && <OverviewTab stats={stats} />}
        {tab === "Statistics" && <StatisticsTab stats={stats} />}
        {tab === "Advanced" && <AdvancedTab session={session} stats={stats} />}
      </div>
    </div>
  );
}
