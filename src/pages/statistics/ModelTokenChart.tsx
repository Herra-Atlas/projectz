import type { ReactNode } from "react";
import { Legend, Line, LineChart, CartesianGrid, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import { formatTokens } from "../../features/chat/sessionStats";
import { modelLabel, type ModelNames } from "../../features/statistics/modelLabels";
import { formatBucket, type AxisGranularity } from "../../features/statistics/timeframes";
import type { ModelBucket } from "../../features/statistics/types";

/** One colour per model, taken from the `--series-*` tokens in `App.css` so the
    chart restyles with the theme rather than carrying its own palette. The count
    matches `MODEL_SERIES_LIMIT` on the Rust side, which caps the series sent. */
const SERIES_COLORS = [
  "var(--series-1)",
  "var(--series-2)",
  "var(--series-3)",
  "var(--series-4)",
  "var(--series-5)",
  "var(--series-6)",
];

/** The busiest model gets the first colour, so the line the reader is most likely
    looking for is the strongest one. Models are ranked by total output over the
    period, which is the backend's own ordering for the series it sends. */
function rankModels(rows: ModelBucket[]): string[] {
  const totals = new Map<string, number>();
  for (const row of rows) {
    totals.set(row.model_id, (totals.get(row.model_id) ?? 0) + row.completion_tokens);
  }
  return [...totals.entries()].sort((a, b) => b[1] - a[1]).map(([model]) => model);
}

/** What Recharts hands a custom tooltip, narrowed to what this chart reads. */
type TooltipProps = {
  active?: boolean;
  label?: ReactNode;
  payload?: ReadonlyArray<{ payload?: Record<string, number | string> }>;
};

/**
 * The hover card for the per-model chart.
 *
 * **Only models that spent something in this bucket are listed.** A model that
 * ranked over the whole period still has a zero column in every bucket it did
 * not appear in, and printing `in 0 · out 0 · 0 req` for each of those buried
 * the models the reader was actually looking for. Filtering happens here rather
 * than in the data because a missing column would break the line series.
 *
 * The figure leads with the **total**, which is what the chart plots, then the
 * input/output split and the request count. Before this the card showed only the
 * split, so the number the line was drawing had to be added up by hand.
 */
function ModelTooltip({ active, label, payload, labels, models }: TooltipProps & { labels: Map<string, string>; models: string[] }) {
  if (!active || !payload?.length) return null;
  const point = payload[0]?.payload;
  if (!point) return null;
  const entries = models
    .map((model) => ({
      model,
      total: Number(point[model] ?? 0),
      input: Number(point[`${model}__in`] ?? 0),
      output: Number(point[`${model}__out`] ?? 0),
      requests: Number(point[`${model}__req`] ?? 0),
    }))
    .filter((entry) => entry.total > 0)
    .sort((left, right) => right.total - left.total);
  if (entries.length === 0) return null;
  return (
    <div className="rounded-lg border border-[var(--line)] bg-[var(--rail)] px-2.5 py-2 shadow-[0_10px_30px_-18px_rgba(0,0,0,0.6)]">
      <p className="mb-1 text-[11px] text-[var(--muted)]">{label}</p>
      <ul className="flex flex-col gap-1.5">
        {entries.map((entry) => (
          <li key={entry.model}>
            <span className="flex items-center gap-1.5 text-[12px] text-[var(--text)]">
              <span
                className="size-2 shrink-0 rounded-full"
                style={{ background: SERIES_COLORS[models.indexOf(entry.model) % SERIES_COLORS.length] }}
              />
              {labels.get(entry.model) ?? entry.model}
            </span>
            <span className="block pl-3.5 text-[11px] tabular-nums text-[var(--muted)]">
              Total {formatTokens(entry.total)} · in {formatTokens(entry.input)} · out {formatTokens(entry.output)} · {entry.requests.toLocaleString()} req
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}

type ModelTokenChartProps = {
  timeline: ModelBucket[];
  axis: AxisGranularity;
  /** Endpoint and local-model names, so the legend prints a name rather than a
      stored id — a local model's id is a UUID. */
  names: ModelNames;
};

/**
 * The same series as `TokenChart`, split by model.
 *
 * Plots input and output together per model as a stacked area, rather than two
 * lines per model: six models times two series is twelve lines, which no one can
 * follow across the chart. The stack keeps the total readable -- it is the same
 * figure the aggregate view draws -- while still showing which model produced
 * it.
 *
 * Gaps are read from the buckets the backend returned rather than being
 * interpolated, so a model quiet in a bucket reads as zero instead of the line
 * jumping across the gap.
 */
export default function ModelTokenChart({ timeline, axis, names }: ModelTokenChartProps) {
  const models = rankModels(timeline);
  // The series are keyed by id, but a legend entry has to be a name. Resolved
  // from the rows themselves, so the chart needs no registry of its own and a
  // model that answered is always named the way the table names it.
  const labels = new Map(timeline.map((row) => [row.model_id, modelLabel(row, names)]));
  if (models.length === 0) {
    return <p className="px-4 py-12 text-center text-sm text-[var(--muted)]">No model token usage recorded in this period.</p>;
  }

  // One row per bucket, with a column per model. Recharts stacks an area chart by
  // column order, so the data is reshaped rather than passed as-is.
  const buckets = [...new Set(timeline.map((row) => row.bucket))].sort();
  const data = buckets.map((bucket) => {
    const point: Record<string, number | string> = { bucket, label: formatBucket(bucket, axis) };
    for (const model of models) {
      point[model] = 0;
      // The line plots one total per model, but the tooltip can afford the
      // breakdown the line cannot: these columns are read by the formatter only,
      // never drawn as series.
      point[`${model}__in`] = 0;
      point[`${model}__out`] = 0;
      point[`${model}__req`] = 0;
    }
    for (const row of timeline) {
      if (row.bucket !== bucket) continue;
      point[row.model_id] = row.prompt_tokens + row.completion_tokens;
      point[`${row.model_id}__in`] = row.prompt_tokens;
      point[`${row.model_id}__out`] = row.completion_tokens;
      point[`${row.model_id}__req`] = row.requests;
    }
    return point;
  });

  const totals = data.reduce(
    (sum, point) => sum + models.reduce((inner, model) => inner + (point[model] as number), 0),
    0,
  );
  if (totals === 0) {
    return <p className="px-4 py-12 text-center text-sm text-[var(--muted)]">No model token usage recorded in this period.</p>;
  }

  return (
    <div className="h-[260px] w-full px-1 py-3">
      <ResponsiveContainer width="100%" height="100%">
        <LineChart data={data} margin={{ top: 4, right: 12, bottom: 0, left: 0 }}>
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
            isAnimationActive={false}
            // A custom card rather than `formatter`: Recharts renders one entry
            // per series on the row, and there is no way to drop an entry from
            // the formatter alone. The card also says the total and keeps the
            // series colours, which the formatter could not.
            content={(props) => <ModelTooltip {...(props as unknown as TooltipProps)} labels={labels} models={models} />}
          />
          <Legend
            verticalAlign="bottom"
            height={28}
            iconType="plainline"
            iconSize={14}
            wrapperStyle={{ fontSize: 11, color: "var(--muted)" }}
            formatter={(value) => labels.get(String(value)) ?? String(value)}
          />
          {models.map((model, index) => (
            <Line
              key={model}
              type="monotone"
              dataKey={model}
              name={model}
              stroke={SERIES_COLORS[index % SERIES_COLORS.length]}
              strokeWidth={2}
              dot={false}
              activeDot={{ r: 3 }}
              // Animation is off so a window resize redraws the finished series
              // instead of replaying its entrance from the left.
              isAnimationActive={false}
            />
          ))}
        </LineChart>
      </ResponsiveContainer>
    </div>
  );
}