import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { DEFAULT_TIMEFRAME, rangeStart, timeframe, type TimeframeKey } from "./timeframes";
import type { UsageReport } from "./types";

/**
 * Loads the usage report for one timeframe, and again whenever the timeframe
 * changes or the caller bumps `refreshKey` (after a new reply, so the page
 * reflects recent activity).
 *
 * The previous report is kept while the next one loads so switching timeframe
 * does not blank the page, and a request that loses a race is discarded rather
 * than overwriting a newer result.
 */
export function useUsageReport(enabled: boolean, refreshKey: number) {
  const [range, setRange] = useState<TimeframeKey>(DEFAULT_TIMEFRAME);
  const [report, setReport] = useState<UsageReport | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    if (!enabled) return;
    let cancelled = false;
    const current = timeframe(range);
    setLoading(true);
    invoke<UsageReport>("database_usage_report", {
      range: current.key,
      since: rangeStart(current.days),
      bucketSql: current.bucket,
    })
      .then((result) => {
        if (cancelled) return;
        setReport(result);
        setError("");
      })
      .catch((reason: unknown) => {
        if (cancelled) return;
        setError("Statistics could not be read from the database.");
        console.error("Usage report failed:", reason);
      })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [enabled, range, refreshKey]);

  return { report, loading, error, range, setRange };
}
