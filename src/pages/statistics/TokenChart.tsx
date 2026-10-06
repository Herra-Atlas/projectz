import { Area, AreaChart, CartesianGrid, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import { formatTokens } from "../../features/chat/sessionStats";
import { fillTimeline, formatBucket, type AxisGranularity } from "../../features/statistics/timeframes";
import type { TokenBucket } from "../../features/statistics/types";

/** Series colours are CSS variables, not literals, so the chart follows the
    tokens in `App.css` and needs no JavaScript to restyle. Recharts reads
    `var(--x)` straight through to the SVG. */
const PROMPT_COLOR = "var(--accent)";
const COMPLETION_COLOR = "var(--muted)";

type Point = { bucket: string; prompt: number; completion: number; label: string };

type TokenChartProps = {
  timeline: TokenBucket[];
  axis: AxisGranularity;
  days: number;
};

export default function TokenChart({ timeline, axis, days }: TokenChartProps) {
  const data: Point[] = fillTimeline(timeline, axis, days).map((entry) => ({
    bucket: entry.bucket,
    prompt: entry.prompt_tokens,
    completion: entry.completion_tokens,
    label: formatBucket(entry.bucket, axis),
  }));

  const totals = data.reduce((sum, point) => sum + point.prompt + point.completion, 0);
  if (totals === 0) {
    return (
      <p className="px-4 py-12 text-center text-sm text-[var(--muted)]">
        No token usage recorded in this period.
      </p>
    );
  }

  return (
    // A fixed height keeps the layout from jumping when the chart mounts, and
    // ResponsiveContainer handles the width so the panel can sit in a grid.
    <div className="h-[260px] w-full px-1 py-3">
      <ResponsiveContainer width="100%" height="100%">
        <AreaChart data={data} margin={{ top: 4, right: 12, bottom: 0, left: 0 }}>
          <defs>
            <linearGradient id="pz-prompt-fill" x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" stopColor={PROMPT_COLOR} stopOpacity={0.28} />
              <stop offset="100%" stopColor={PROMPT_COLOR} stopOpacity={0.02} />
            </linearGradient>
            <linearGradient id="pz-completion-fill" x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" stopColor={COMPLETION_COLOR} stopOpacity={0.24} />
              <stop offset="100%" stopColor={COMPLETION_COLOR} stopOpacity={0.02} />
            </linearGradient>
          </defs>
          <CartesianGrid stroke="var(--line)" strokeDasharray="2 4" vertical={false} />
          <XAxis
            dataKey="label"
            stroke="var(--quiet)"
            tick={{ fontSize: 11 }}
            tickLine={false}
            axisLine={{ stroke: "var(--line)" }}
            interval="preserveStartEnd"
            minTickGap={16}
          />
          <YAxis
            stroke="var(--quiet)"
            tick={{ fontSize: 11 }}
            tickLine={false}
            axisLine={false}
            width={44}
            tickFormatter={(value: number) => formatTokens(value)}
          />
          <Tooltip
            cursor={{ stroke: "var(--line)" }}
            contentStyle={{
              background: "var(--rail)",
              border: "1px solid var(--line)",
              borderRadius: 8,
              fontSize: 12,
            }}
            labelStyle={{ color: "var(--muted)" }}
            formatter={(value, name) => [formatTokens(Number(value)), name === "prompt" ? "Input" : "Output"]}
          />
          <Area type="monotone" dataKey="prompt" name="prompt" stroke={PROMPT_COLOR} strokeWidth={2} fill="url(#pz-prompt-fill)" dot={false} activeDot={{ r: 3 }} />
          <Area type="monotone" dataKey="completion" name="completion" stroke={COMPLETION_COLOR} strokeWidth={2} fill="url(#pz-completion-fill)" dot={false} activeDot={{ r: 3 }} />
        </AreaChart>
      </ResponsiveContainer>
    </div>
  );
}
