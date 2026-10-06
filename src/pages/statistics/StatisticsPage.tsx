import { useMemo, useState } from "react";
import type { ChatSessionHeader } from "../../features/chat/types";
import { timeframe } from "../../features/statistics/timeframes";
import { useUsageReport } from "../../features/statistics/useUsageReport";
import { useModelRegistry } from "../../features/models/useModelRegistry";
import { Panel } from "./FigureCard";
import FigureRow from "./FigureRow";
import TokenChart from "./TokenChart";
import ModelTokenChart from "./ModelTokenChart";
import ActivityChart from "./ActivityChart";
import { ModelTable, SessionTable, modelGroups } from "./UsageTables";
import { PanelPager } from "./PanelPager";
import TimeframeSwitcher from "./TimeframeSwitcher";

/** Read-only usage summary across every conversation.
 *
 * The header states the period and the figures; the body plots tokens over time
 * and breaks the same period down by activity, model, and conversation. The
 * timeframe control is the only input, and it re-queries rather than filtering
 * the frontend, so the cost does not grow with history.
 *
 * `sessions` is the loaded conversation list, used to make a row clickable: the
 * report carries only per-conversation totals, so a row naming a conversation
 * that is not in the list has nothing to open. Headers are enough for that test,
 * which is why the page never needed the transcripts.
 *
 * `onOpenOverview` raises the request rather than fetching. The overview is
 * rendered once by `App`, which owns the transcript read, so opening it from here
 * and from the sidebar are the same action.
 */
export default function StatisticsPage({
  refreshKey = 0,
  sessions = [],
  onOpenOverview,
}: {
  refreshKey?: number;
  sessions?: ChatSessionHeader[];
  onOpenOverview: (id: string) => void;
}) {
  const { report, loading, error, range, setRange } = useUsageReport(true, refreshKey);
  const current = timeframe(range);
  // Only for deciding which rows are clickable. The open modal lives in `App`.
  const known = new Set(sessions.map((session) => session.id));

  // The token panel pages between the aggregate view and the per-model one.
  // Both draw the same underlying series, so this is a change of drawing rather
  // than of data -- the aggregate stays the default because it is the one that
  // answers "what did this period cost", which is what most visits want.
  const [tokenView, setTokenView] = useState(0);
  const [modelView, setModelView] = useState(0);

  // Endpoint names, so a provider view is labelled the way the user named the
  // provider in Settings rather than by the internal id stored on each message.
  const { endpoints } = useModelRegistry(refreshKey);
  const providerNames = useMemo(() => new Map(endpoints.map((endpoint) => [endpoint.id, endpoint.name])), [endpoints]);
  const groups = useMemo(() => modelGroups(report?.models ?? [], providerNames), [report?.models, providerNames]);
  // A timeframe switch can report fewer providers than the previous one, which
  // would leave the view index past the end.
  const modelGroup = groups[Math.min(modelView, groups.length - 1)];

  const tokenViews = ["Totals", "Per model"];

  return (
    <main className="flex min-h-0 flex-1 flex-col">
      <header className="flex min-h-[68px] flex-wrap items-center justify-between gap-3 border-b border-[var(--line)] px-5 sm:px-7">
        <div className="min-w-0">
          <h1 className="text-sm font-semibold">Statistics</h1>
          <p className="text-xs text-[var(--quiet)]">
            {report ? `Usage since ${report.since} · times in UTC` : "Usage across all conversations"}
          </p>
        </div>
        <TimeframeSwitcher value={range} onChange={setRange} />
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto p-5 sm:p-7">
        {error && <p role="alert" className="mb-5 text-sm text-[var(--danger)]">{error}</p>}

        <FigureRow report={report} />

        <div className="mt-5">
          <Panel
            title={tokenView === 0 ? "Tokens over time" : "Tokens per model"}
            action={
              <>
                {loading && <span className="mr-2 text-[11px] text-[var(--quiet)]">Loading…</span>}
                <PanelPager count={tokenViews.length} index={tokenView} labels={tokenViews} onChange={setTokenView} what="token view" />
              </>
            }
          >
            {!report ? <p className="px-4 py-12 text-center text-sm text-[var(--muted)]">Loading…</p>
              : tokenView === 0
                ? <TokenChart timeline={report.timeline} axis={current.axis} days={current.days} />
                : <ModelTokenChart timeline={report.model_timeline} axis={current.axis} />}
          </Panel>
        </div>

        <div className="mt-3 grid gap-3 lg:grid-cols-2">
          <Panel title="Activity">
            {report ? <ActivityChart report={report} /> : <p className="px-4 py-12 text-center text-sm text-[var(--muted)]">Loading…</p>}
          </Panel>
          <Panel
            title={modelGroup ? `Models · ${modelGroup.label}` : "Models"}
            action={
              <PanelPager
                count={groups.length}
                index={Math.min(modelView, groups.length - 1)}
                labels={groups.map((group) => group.label)}
                onChange={setModelView}
                what="model view"
              />
            }
          >
            {report ? <ModelTable group={modelGroup} /> : <p className="px-4 py-12 text-center text-sm text-[var(--muted)]">Loading…</p>}
          </Panel>
        </div>

        <div className="mt-3">
          <Panel title="Conversations">
            {report
              // A row is clickable only when its conversation is loaded, because
              // the report carries totals and not the transcript the overview
              // needs. The modal itself is `App`'s.
              ? <SessionTable sessions={report.session_usage.filter((row) => known.has(row.id))} onOpen={onOpenOverview} />
              : <p className="px-4 py-12 text-center text-sm text-[var(--muted)]">Loading…</p>}
          </Panel>
        </div>
      </div>
    </main>
  );
}