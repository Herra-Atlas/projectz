/**
 * A byte count a reader can compare at a glance.
 *
 * One decimal below ten and none above it: "4.1 GB" and "63 MB" are the
 * precision those numbers are useful at, and "4.09 GB" is noise.
 */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "—";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value >= 10 || unit === 0 ? value.toFixed(0) : value.toFixed(1)} ${units[unit]}`;
}

/**
 * A transfer rate, in the same units as the sizes it divides.
 *
 * Zero is stated as zero rather than hidden: a download that is running at 0 B/s
 * is stalled, and that is worth seeing.
 */
export function formatSpeed(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B/s";
  return `${formatBytes(bytes)}/s`;
}
