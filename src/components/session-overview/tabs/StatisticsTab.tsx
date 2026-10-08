import { ChartNoAxesColumn, Clock, Gauge, Layers } from "lucide-react";
import { Suspense, lazy } from "react";
import { formatTokens, type SessionStats } from "../../../features/chat/sessionStats";
import { formatDuration } from "../../../features/chat/duration";
import { Group, Stat, StatGrid } from "../StatGroup";

/** Recharts is the heaviest dependency in the app, and the overview panel is
    reachable from the sidebar on every screen. Loading the charts lazily keeps
    it out of the chat bundle; the fallback is the height the chart will take,
    so the tab does not jump when the chunk lands. */
const ReplyTokensChart = lazy(() => import("../ReplyCharts").then((module) => ({ default: module.ReplyTokensChart })));
const ReplyRateChart = lazy(() => import("../ReplyCharts").then((module) => ({ default: module.ReplyRateChart })));
const ContextChart = lazy(() => import("../ReplyCharts").then((module) => ({ default: module.ContextChart })));

function ChartSlot({ children }: { children: React.ReactNode }) {
  return <Suspense fallback={<div className="h-[150px]" />}>{children}</Suspense>;
}

/** Token and timing totals, plus a chart per reply.
 *
 * Separate from the opening view so the headline rate there is not competing
 * with a dozen numbers of the same weight. The charts answer questions the
 * totals cannot: which reply was the long one, and whether the speed held up as
 * the context grew.
 */
export default function StatisticsTab({ stats }: { stats: SessionStats }) {
  return (
    <>
      <Group icon={<Layers size={13} />} title="Tokens">
        <StatGrid>
          <Stat label="Total" value={formatTokens(stats.totalTokens)} hint="prompt + completion" />
          <Stat label="Prompt" value={formatTokens(stats.promptTokens)} hint="sent to models" />
          <Stat label="Completion" value={formatTokens(stats.completionTokens)} hint="generated" />
        </StatGrid>
      </Group>

      <Group icon={<Layers size={13} />} title="Context">
        <ChartSlot><ContextChart stats={stats} /></ChartSlot>
        <p className="mt-1.5 text-[11px] text-[var(--quiet)]">
          Tokens sent at each turn, oldest first. Hover a point for its input and
          output. A rising line is the conversation accumulating.
        </p>
      </Group>

      {/* Charted rather than tabulated: the shape across replies is the point,
          and a per-reply table would be taller than the panel. */}
      <Group icon={<ChartNoAxesColumn size={13} />} title="Tokens per reply">
        <ChartSlot><ReplyTokensChart stats={stats} /></ChartSlot>
        <p className="mt-1.5 text-[11px] text-[var(--quiet)]">
          Input and output for each reply, oldest first. Peaks show the exchanges
          that carried the most context.
        </p>
      </Group>

      <Group icon={<Gauge size={13} />} title="Speed per reply">
        <ChartSlot><ReplyRateChart stats={stats} /></ChartSlot>
        <p className="mt-1.5 text-[11px] text-[var(--quiet)]">
          {stats.hasEstimatedRates
            ? "Replies with an inferred rate are left out, so the line only shows measured speed."
            : "Generation speed for each reply that reported timing."}
        </p>
      </Group>

      <Group icon={<Clock size={13} />} title="Timing">
        <StatGrid>
          <Stat
            label="Generation time"
            value={stats.generationSeconds > 0 ? formatDuration(stats.generationSeconds) : "—"}
            hint="total across replies"
          />
          <Stat
            label="Prompt time"
            value={stats.promptSeconds > 0 ? formatDuration(stats.promptSeconds) : "—"}
            hint="total across replies"
          />
          <Stat
            label="Replies timed"
            value={String(stats.measuredReplies)}
            hint={`of ${stats.assistantMessages} ${stats.assistantMessages === 1 ? "reply" : "replies"}`}
          />
        </StatGrid>
      </Group>
    </>
  );
}
