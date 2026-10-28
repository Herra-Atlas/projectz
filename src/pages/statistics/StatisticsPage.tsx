import { useMemo, useState } from "react";
import type { ChatSessionHeader } from "../../features/chat/types";
import { formatTokens } from "../../features/chat/sessionStats";
import { timeframe } from "../../features/statistics/timeframes";
import { useUsageReport } from "../../features/statistics/useUsageReport";
import { useModelRegistry } from "../../features/models/useModelRegistry";
import { Panel } from "./FigureCard";
import FigureRow from "./FigureRow";
import TokenChart from "./TokenChart";
import ModelTokenChart from "./ModelTokenChart";
import { ToolBars } from "../../components/statistics/ToolBars";
import { ModelTable, SessionTable, modelGroups } from "./UsageTables";
import { PanelTabs } from "./PanelTabs";
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
 * rendered once by the right panel, which reads the transcript, so opening it
 * from here and from the sidebar are the same action.
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
  // Only for deciding which rows are clickable. The overview view lives in the
  // right panel, opened by the request `App` raises.
  const known = new Set(sessions.map((session) => session.id));

  // The token panel switches between the aggregate view, the per-model one and
  // taste's own runs. All three draw the same series, so this is a change of
  // drawing rather than of data -- the aggregate stays the default because it is
  // the one that answers "what did this period cost", which is what most visits
  // want. Every view is named in the header, so the switch is one press and the
  // alternatives are readable before it.
  const [tokenView, setTokenView] = useState(0);
  const [modelView, setModelView] = useState(0);

  // Endpoint names, so a provider view is labelled the way the user named the
  // provider in Settings rather than by the internal id stored on each message.
  const { endpoints, localModels } = useModelRegistry(refreshKey);
  const providerNames = useMemo(() => new Map(endpoints.map((endpoint) => [endpoint.id, endpoint.name])), [endpoints]);
  // A local model's stored id is the UUID its row was given, and its name lives
  // only here. Without this the table and the chart would print the id.
  const localModelNames = useMemo(() => new Map(localModels.map((model) => [model.id, model.name])), [localModels]);
  const names = useMemo(() => ({ providers: providerNames, local: localModelNames }), [providerNames, localModelNames]);
  const groups = useMemo(() => modelGroups(report?.models ?? [], names), [report?.models, names]);
  // A timeframe switch can report fewer providers than the previous one, which
  // would leave the view index past the end.
  const modelIndex = Math.min(modelView, groups.length - 1);
  const modelGroup = groups[modelIndex];

  // The tab labels are the view titles themselves, so the header names what is
  // on screen rather than stating a position in a list.
  const tokenViews = ["Tokens over time", "Tokens per model", "Taste tokens per model"];

  return (
    <main className="flex min-h-0 flex-1 flex-col">
      <header className="flex min-h-[68px] flex-wrap items-center justify-between gap-3 border-b border-[var(--line)] px-5 sm:px-7">
        <div className="min-w-0">
          <h1 className="text-sm font-semibold">Statistics</h1>
          <p className="text-xs text-[var(--quiet)]">
            {report
              ? report.range === "all"
                ? "All time · times in UTC"
                : `Usage since ${report.since} · times in UTC`
              : "Usage across all conversations"}
          </p>
        </div>
        <TimeframeSwitcher value={range} onChange={setRange} />
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto p-5 sm:p-7">
        {error && <p role="alert" className="mb-5 text-sm text-[var(--danger)]">{error}</p>}

        <FigureRow report={report} axis={current.axis} />

        <div className="mt-5">
          <Panel
            tabs={
              <PanelTabs
                what="token view"
                tabs={tokenViews.map((label, index) => ({
                  label,
                  active: tokenView === index,
                  onSelect: () => setTokenView(index),
                }))}
              />
            }
            action={loading ? <span className="text-[11px] text-[var(--quiet)]">Loading…</span> : null}
          >
            {!report ? <p className="px-4 py-12 text-center text-sm text-[var(--muted)]">Loading…</p>
              : tokenView === 0
                ? <TokenChart timeline={report.timeline} axis={current.axis} days={current.days} />
                : tokenView === 1
                  ? <ModelTokenChart timeline={report.model_timeline} axis={current.axis} names={names} />
                  : <>
                      <p className="px-4 pt-3 text-[11px] text-[var(--quiet)]">
                        {formatTokens(report.taste_total_tokens)} tokens over {report.taste_runs === 1 ? "1 run" : `${report.taste_runs} runs`} since {report.since}.
                      </p>
                      <ModelTokenChart timeline={report.taste_timeline} axis={current.axis} names={names} />
                    </>}
          </Panel>
        </div>

        <div className="mt-3 grid gap-3 lg:grid-cols-2">
          {/* What the agent did, which no other panel answers. The old three bars
              here repeated the cards above and counted a message role the app
              never writes, so the panel is now the tool calls themselves. */}
          <Panel title="Tools">
            {report
              ? <ToolBars tools={report.tools} empty="No tool calls in this period." />
              : <p className="px-4 py-12 text-center text-sm text-[var(--muted)]">Loading…</p>}
          </Panel>
          <Panel
            title="Models"
            tabs={
              <PanelTabs
                what="model view"
                tabs={groups.map((group, index) => ({
                  label: group.label,
                  active: modelIndex === index,
                  onSelect: () => setModelView(index),
                }))}
              />
            }
          >
            {report ? <ModelTable group={modelGroup} names={names} /> : <p className="px-4 py-12 text-center text-sm text-[var(--muted)]">Loading…</p>}
          </Panel>
        </div>

        <div className="mt-3">
          <Panel title="Conversations">
            {report
              // A row is clickable only when its conversation is loaded, because
              // the report carries totals and not the transcript the overview
              // needs. The view itself is the right panel's.
              ? <SessionTable sessions={report.session_usage.filter((row) => known.has(row.id))} onOpen={onOpenOverview} />
              : <p className="px-4 py-12 text-center text-sm text-[var(--muted)]">Loading…</p>}
          </Panel>
        </div>
      </div>
    </main>
  );
}