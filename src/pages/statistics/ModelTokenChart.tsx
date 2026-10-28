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
            contentStyle={{
              background: "var(--rail)",
              border: "1px solid var(--line)",
              borderRadius: 8,
              fontSize: 12,
            }}
            labelStyle={{ color: "var(--muted)" }}
            formatter={(value, name, item) => {
              const model = String(name);
              // The row the pointer is over, so the hovered model's split can be
              // read without adding three series per model to the chart.
              const row = item?.payload as Record<string, number | string> | undefined;
              const input = row?.[`${model}__in`];
              const output = row?.[`${model}__out`];
              const requests = row?.[`${model}__req`];
              const detail = typeof input === "number" && typeof output === "number" && typeof requests === "number"
                ? `in ${formatTokens(input)} · out ${formatTokens(output)} · ${requests.toLocaleString()} req`
                : formatTokens(Number(value));
              return [detail, labels.get(model) ?? model];
            }}
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