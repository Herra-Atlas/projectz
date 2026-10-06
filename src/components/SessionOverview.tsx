import { useEffect, useRef, useState } from "react";
import { X } from "lucide-react";
import { computeSessionStats, formatTimestamp, type SessionStats } from "../features/chat/sessionStats";
import type { ChatSession } from "../features/chat/types";
import TabStrip, { type SessionOverviewTab } from "./session-overview/TabStrip";
import OverviewTab from "./session-overview/tabs/OverviewTab";
import StatisticsTab from "./session-overview/tabs/StatisticsTab";
import AdvancedTab from "./session-overview/tabs/AdvancedTab";

type SessionOverviewProps = {
  session: ChatSession | null;
  onClose: () => void;
};

/** Read-only detail view for a single conversation.
 *
 * Figures come from `computeSessionStats`, which aggregates only replies that
 * actually reported timing. Anything inferred is labelled as an estimate so a
 * derived number is never presented as a measured one.
 *
 * The body is split into tabs under the header: Overview leads with the one
 * figure worth reading at a glance, Statistics holds the token and timing
 * totals, and Advanced holds the record's own metadata. Each tab is a separate
 * file so adding a section does not grow this shell.
 */
export default function SessionOverview({ session, onClose }: SessionOverviewProps) {
  const [tab, setTab] = useState<SessionOverviewTab>("Overview");
  const closeRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!session) return;
    closeRef.current?.focus();
    const onKeyDown = (event: KeyboardEvent) => { if (event.key === "Escape") onClose(); };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [session, onClose]);

  // A new session should open on the leading tab rather than inherit the last
  // one the user was reading, since the figures are not comparable.
  useEffect(() => setTab("Overview"), [session?.id]);

  if (!session) return null;
  const stats = computeSessionStats(session);

  return (
    <div className="fixed inset-0 z-[140] grid place-items-center bg-black/65 p-4" onClick={onClose}>
      <section
        role="dialog"
        aria-modal="true"
        aria-labelledby="session-overview-title"
        className="flex max-h-[85vh] w-full max-w-lg flex-col overflow-hidden rounded-xl border border-[var(--line)] bg-[var(--panel)] shadow-2xl"
        onClick={(event) => event.stopPropagation()}
        onKeyDown={(event) => { if (event.key === "Escape") onClose(); }}
      >
        <header className="flex shrink-0 items-start justify-between gap-4 border-b border-[var(--line)] px-5 py-4">
          <div className="min-w-0">
            <h2 id="session-overview-title" className="truncate text-base font-semibold" title={session.title}>{session.title}</h2>
            <p className="mt-0.5 text-xs text-[var(--quiet)]">
              {formatTimestamp(stats.createdAt)} &rarr; {formatTimestamp(stats.updatedAt)}
            </p>
          </div>
          <button ref={closeRef} type="button" onClick={onClose} className="grid size-8 shrink-0 place-items-center rounded-md text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]" aria-label="Close overview"><X size={17} /></button>
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
      </section>
    </div>
  );
}

export type { SessionStats };
