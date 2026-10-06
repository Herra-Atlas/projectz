/** The statistics page's timeframes.
 *
 * `bucket` is the SQLite expression that groups the token timeline, and must
 * stay identical to one of the entries in `TIMEFRAME_BUCKETS` on the Rust side
 * (`database_usage_report` rejects anything else). Keeping the literal here
 * means the SQL is a fixed constant rather than anything built from input.
 *
 * All stored timestamps are UTC (`new Date().toISOString()`), so buckets are
 * UTC too and the axis says so. Localising would mean shifting every bucket
 * after the fact, which misplaces points near midnight.
 */

export const TIMEFRAMES = [
  { key: "day", label: "Day", days: 1, bucket: "substr(created_at,1,13)", axis: "hour" },
  { key: "week", label: "Week", days: 7, bucket: "substr(created_at,1,10)", axis: "day" },
  { key: "month", label: "Month", days: 30, bucket: "substr(created_at,1,10)", axis: "day" },
  { key: "quarter", label: "3 months", days: 90, bucket: "substr(created_at,1,7)", axis: "month" },
  { key: "year", label: "Year", days: 365, bucket: "substr(created_at,1,4)", axis: "year" },
] as const;

export type TimeframeKey = (typeof TIMEFRAMES)[number]["key"];
export type AxisGranularity = (typeof TIMEFRAMES)[number]["axis"];

export const DEFAULT_TIMEFRAME: TimeframeKey = "month";

export const timeframe = (key: TimeframeKey) =>
  TIMEFRAMES.find((entry) => entry.key === key) ?? TIMEFRAMES[2];

/** Inclusive lower bound as `YYYY-MM-DD`, `days` before today. */
export const rangeStart = (days: number, now = new Date()) => {
  const start = new Date(now);
  start.setDate(start.getDate() - (days - 1));
  return start.toISOString().slice(0, 10);
};

/**
 * Turns a bucket key into a short axis label.
 *
 * Day buckets read as `1 Oct`; hour buckets drop the date because a single day
 * rarely spans more than a handful of them; month and year buckets keep only the
 * part that changes. Returns the key unchanged if it is not the expected shape,
 * so an unexpected value shows rather than renders as "Invalid Date".
 */
export const formatBucket = (bucket: string, axis: AxisGranularity) => {
  if (axis === "year") return bucket;
  if (axis === "month") {
    const parsed = new Date(`${bucket}-01T00:00:00Z`);
    if (Number.isNaN(parsed.getTime())) return bucket;
    return parsed.toLocaleDateString(undefined, { month: "short", timeZone: "UTC" });
  }
  if (axis === "hour") {
    const parsed = new Date(`${bucket}:00:00Z`);
    if (Number.isNaN(parsed.getTime())) return bucket;
    return parsed.toLocaleTimeString(undefined, { hour: "numeric", timeZone: "UTC" });
  }
  const parsed = new Date(`${bucket}T00:00:00Z`);
  if (Number.isNaN(parsed.getTime())) return bucket;
  return parsed.toLocaleDateString(undefined, { day: "numeric", month: "short", timeZone: "UTC" });
};

/**
 * Fills gaps so a quiet day reads as zero rather than stretching the line
 * across it. The chart plots consecutive points, so a missing bucket would
 * otherwise imply activity that never happened.
 */
export const fillTimeline = (
  buckets: { bucket: string; prompt_tokens: number; completion_tokens: number }[],
  axis: AxisGranularity,
  days: number,
) => {
  const byKey = new Map(buckets.map((entry) => [entry.bucket, entry]));
  const step = axis === "hour" ? 3_600_000 : axis === "day" ? 86_400_000 : 0;
  const start = new Date(`${(buckets[0]?.bucket ?? rangeStart(days)).slice(0, 10)}T00:00:00Z`);

  if (axis === "month" || axis === "year" || !Number.isFinite(start.getTime())) {
    return [...buckets].sort((a, b) => a.bucket.localeCompare(b.bucket));
  }

  const output: { bucket: string; prompt_tokens: number; completion_tokens: number }[] = [];
  for (let index = 0; index < days; index += 1) {
    const at = new Date(start.getTime() + index * step);
    const key = axis === "hour" ? at.toISOString().slice(0, 13) : at.toISOString().slice(0, 10);
    const found = byKey.get(key);
    output.push({ bucket: key, prompt_tokens: found?.prompt_tokens ?? 0, completion_tokens: found?.completion_tokens ?? 0 });
  }
  return output;
};
