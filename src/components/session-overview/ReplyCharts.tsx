import { Area, AreaChart, Bar, BarChart, CartesianGrid, Line, LineChart, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import { formatDuration, formatTokens, type SessionStats } from "../../features/chat/sessionStats";

/** Charts for a single conversation's replies.
 *
 * The modal is not lazy-loaded on its own, and it is opened from several
 * places, so Recharts is imported here directly rather than duplicating the
 * statistics page's own components — the series differ per conversation and the
 * axis is positional rather than dated.
 *
 * Colours are CSS variables from `App.css`, so these follow the app's tokens
 * and restyle without a re-render.
 */

const PROMPT_COLOR = "var(--accent)";
const COMPLETION_COLOR = "var(--muted)";
const RATE_COLOR = "var(--accent)";

const AXIS = { stroke: "var(--quiet)", fontSize: 10 } as const;
const GRID = "var(--line)";

const TOOLTIP_STYLE = {
  background: "var(--rail)",
  border: "1px solid var(--line)",
  borderRadius: 8,
  fontSize: 12,
} as const;

/** Input against output tokens, one pair of bars per reply. */
export function ReplyTokensChart({ stats }: { stats: SessionStats }) {
  if (stats.replies.length === 0) return <ChartEmpty />;

  const data = stats.replies.map((reply) => ({
    label: `#${reply.index}`,
    prompt: reply.promptTokens,
    completion: reply.completionTokens,
  }));

  return (
    <div className="h-[150px] w-full">
      <ResponsiveContainer width="100%" height="100%">
        <BarChart data={data} margin={{ top: 4, right: 8, bottom: 0, left: 0 }} barGap={2}>
          <CartesianGrid stroke={GRID} strokeDasharray="2 4" vertical={false} />
          <XAxis dataKey="label" tick={AXIS} tickLine={false} axisLine={{ stroke: GRID }} interval={0} />
          <YAxis tick={AXIS} tickLine={false} axisLine={false} width={40} tickFormatter={(value: number) => formatTokens(value)} />
          <Tooltip
            cursor={{ fill: "color-mix(in srgb, var(--raised) 60%, transparent)" }}
            contentStyle={TOOLTIP_STYLE}
            labelStyle={{ color: "var(--muted)" }}
            formatter={(value, name) => [formatTokens(Number(value)), name === "prompt" ? "Input" : "Output"]}
          />
          <Bar dataKey="prompt" fill={PROMPT_COLOR} radius={[3, 3, 0, 0]} maxBarSize={14} />
          <Bar dataKey="completion" fill={COMPLETION_COLOR} radius={[3, 3, 0, 0]} maxBarSize={14} />
        </BarChart>
      </ResponsiveContainer>
    </div>
  );
}

/** Generation speed per reply. Replies whose rate was inferred rather than
    reported are dropped, so the line never presents a guess as a measurement. */
export function ReplyRateChart({ stats }: { stats: SessionStats }) {
  const measured = stats.replies.filter((reply) => reply.tokensPerSecond != null);
  if (measured.length === 0) return <ChartEmpty />;

  const data = measured.map((reply) => ({
    label: `#${reply.index}`,
    rate: Number(reply.tokensPerSecond?.toFixed(1)),
    seconds: reply.generationSeconds,
  }));

  return (
    <div className="h-[150px] w-full">
      <ResponsiveContainer width="100%" height="100%">
        <LineChart data={data} margin={{ top: 4, right: 8, bottom: 0, left: 0 }}>
          <CartesianGrid stroke={GRID} strokeDasharray="2 4" vertical={false} />
          <XAxis dataKey="label" tick={AXIS} tickLine={false} axisLine={{ stroke: GRID }} interval={0} />
          <YAxis tick={AXIS} tickLine={false} axisLine={false} width={40} domain={[0, "auto"]} unit="" />
          <Tooltip
            cursor={{ stroke: GRID }}
            contentStyle={TOOLTIP_STYLE}
            labelStyle={{ color: "var(--muted)" }}
            formatter={(value, _name, entry) => {
              const seconds = (entry?.payload as { seconds?: number } | undefined)?.seconds;
              const rate = `${Number(value).toFixed(1)} tok/s`;
              return [seconds ? `${rate} in ${formatDuration(seconds)}` : rate, "Speed"];
            }}
          />
          <Line type="monotone" dataKey="rate" stroke={RATE_COLOR} strokeWidth={2} dot={{ r: 2.5 }} activeDot={{ r: 4 }} />
        </LineChart>
      </ResponsiveContainer>
    </div>
  );
}

/** Context growth: the prompt size sent for each reply, in turn order.
 *
 * The x axis is the reply's position rather than a clock time, because messages
 * carry the session's timestamp rather than their own, so a real timeline would
 * collapse every point onto one date. The tooltip reports both halves of the
 * exchange, since a growing input with a flat output is the shape worth seeing.
 */
export function ContextChart({ stats }: { stats: SessionStats }) {
  if (stats.context.length === 0) return <ChartEmpty />;

  const data = stats.context.map((point) => ({
    label: `#${point.index}`,
    prompt: point.promptTokens,
    completion: point.completionTokens,
  }));

  return (
    <div className="h-[150px] w-full">
      <ResponsiveContainer width="100%" height="100%">
        <AreaChart data={data} margin={{ top: 4, right: 8, bottom: 0, left: 0 }}>
          <defs>
            <linearGradient id="pz-context-fill" x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" stopColor={PROMPT_COLOR} stopOpacity={0.3} />
              <stop offset="100%" stopColor={PROMPT_COLOR} stopOpacity={0.02} />
            </linearGradient>
          </defs>
          <CartesianGrid stroke={GRID} strokeDasharray="2 4" vertical={false} />
          <XAxis dataKey="label" tick={AXIS} tickLine={false} axisLine={{ stroke: GRID }} interval={0} />
          <YAxis tick={AXIS} tickLine={false} axisLine={false} width={40} tickFormatter={(value: number) => formatTokens(value)} />
          <Tooltip
            cursor={{ stroke: GRID }}
            contentStyle={TOOLTIP_STYLE}
            labelStyle={{ color: "var(--muted)" }}
            formatter={(value, name) => [formatTokens(Number(value)), name === "prompt" ? "Input" : "Output"]}
          />
          <Area
            type="monotone"
            dataKey="prompt"
            stroke={PROMPT_COLOR}
            strokeWidth={2}
            fill="url(#pz-context-fill)"
            dot={{ r: 2.5 }}
            activeDot={{ r: 4 }}
          />
        </AreaChart>
      </ResponsiveContainer>
    </div>
  );
}

function ChartEmpty() {
  return <p className="py-10 text-center text-sm text-[var(--quiet)]">No timing reported for this conversation.</p>;
}
