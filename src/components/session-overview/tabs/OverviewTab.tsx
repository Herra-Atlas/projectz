import { ChartNoAxesColumn, MessageSquareText, Timer } from "lucide-react";
import { formatTokens, type SessionStats } from "../../../features/chat/sessionStats";
import { formatDuration } from "../../../features/chat/duration";
import { ElapsedStat } from "../ElapsedStat";
import { Group, Stat, StatGrid } from "../StatGroup";

/** Opening view: the one figure worth reading at a glance, then the shape of
    the exchange. Detail is a tab away so this stays short. */
export default function OverviewTab({ stats }: { stats: SessionStats }) {
  const rate = stats.aggregateTokensPerSecond;
  const rateLabel = rate > 0 ? rate.toFixed(1) : "—";

  return (
    <>
      <Group icon={<ChartNoAxesColumn size={13} />} title="Performance">
        {/* Average speed leads because it is the figure that changes with the
            model or the hardware, so it earns the largest size on the tab. */}
        <div className="flex items-baseline gap-3">
          <p className="text-3xl font-medium tabular-nums tracking-tight text-[var(--text)]">{rateLabel}</p>
          <p className="text-xs text-[var(--quiet)]">
            {rate > 0 ? "tokens per second, averaged across replies" : "no timing reported for this conversation"}
          </p>
        </div>
        <StatGrid>
          <Stat
            label="Prompt speed"
            value={stats.averagePromptTokensPerSecond != null ? `${stats.averagePromptTokensPerSecond.toFixed(1)} tok/s` : "—"}
            hint="average prompt eval"
          />
          <Stat
            label="Generation time"
            value={stats.generationSeconds > 0 ? formatDuration(stats.generationSeconds) : "—"}
            hint="total across replies"
          />
          <Stat
            label="Rate source"
            value={stats.measuredReplies === 0 ? "None" : stats.hasEstimatedRates ? "Mixed" : "Measured"}
            hint={
              stats.measuredReplies === 0
                ? "no timing reported"
                : stats.hasEstimatedRates
                  ? "some inferred from text"
                  : `across ${stats.measuredReplies} ${stats.measuredReplies === 1 ? "reply" : "replies"}`
            }
          />
        </StatGrid>
      </Group>

      {/* What the conversation cost. Message and reply counts are deliberately
          absent: they only ever report "how many turns", which the transcript
          itself already shows. */}
      <Group icon={<MessageSquareText size={13} />} title="Conversation">
        <StatGrid>
          <Stat
            label="Total tokens"
            value={stats.totalTokens > 0 ? formatTokens(stats.totalTokens) : "—"}
            hint={`${formatTokens(stats.promptTokens)} in, ${formatTokens(stats.completionTokens)} out`}
          />
          <ElapsedStat label="Elapsed" since={stats.createdAt} absolute hint="since it was opened" />
          <ElapsedStat label="Last change" since={stats.updatedAt} hint="since the last message" />
        </StatGrid>
      </Group>

      {/*
        How the conversation was spent. Every figure is derived, so a session
        with one exchange collapses the row rather than padding it with zeros.
      */}
      <Group icon={<Timer size={13} />} title="Session">
        <StatGrid>
          <Stat
            label="Searches"
            value={String(stats.searches)}
            hint={stats.searches > 0 ? "web lookups" : "none run"}
          />
          <Stat
            label="Reasoned"
            value={String(stats.reasoningReplies)}
            hint={stats.reasoningReplies > 0 ? `of ${stats.assistantMessages} replies` : "no reasoning used"}
          />
          <Stat
            label="Models"
            value={String(stats.models.length)}
            hint={stats.models.length > 0 ? (stats.models.length === 1 ? stats.models[0] : `${stats.models.length} used`) : "none recorded"}
          />
        </StatGrid>
      </Group>
    </>
  );
}
