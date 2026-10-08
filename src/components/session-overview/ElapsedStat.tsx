import { useEffect, useState } from "react";
import { formatDuration } from "../../features/chat/duration";
import { Stat } from "./StatGroup";

/** A figure counting down from a timestamp, updated once a second.
 *
 * This owns its own timer rather than relying on a parent re-rendering, so the
 * count keeps its own pace and only this subtree is redrawn each second. The
 * tab's other figures and any open chart are left alone.
 */
export function ElapsedStat({
  label,
  since,
  hint,
  absolute = false,
}: {
  label: string;
  /** ISO timestamp to measure from. */
  since: string;
  hint?: string;
  /** When set, the value is a plain duration rather than "… ago". */
  absolute?: boolean;
}) {
  // Re-read once a second so the figure does not silently go stale while the
  // overview panel sits open. `setInterval` is cleared on unmount.
  const [, setTick] = useState(0);
  useEffect(() => {
    const timer = window.setInterval(() => setTick((value) => value + 1), 1000);
    return () => window.clearInterval(timer);
  }, []);

  const from = Date.parse(since);
  if (!Number.isFinite(from)) return <Stat label={label} value="—" hint={hint} />;

  const seconds = Math.max(0, (Date.now() - from) / 1000);
  if (absolute) {
    return <Stat label={label} value={seconds > 0 ? formatDuration(seconds) : "—"} hint={seconds > 0 ? hint : "opened just now"} />;
  }
  // Under a minute the exact count is noise, and a figure that reads "0s ago"
  // while you are looking at it is worse than one that says it plainly.
  if (seconds < 60) return <Stat label={label} value="just now" hint="this conversation is current" />;
  return <Stat label={label} value={`${formatDuration(seconds)} ago`} hint={hint} />;
}
