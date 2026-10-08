/**
 * Human durations for the overview's elapsed figures.
 *
 * A ladder rather than one fixed unit, so a span is read at the scale it
 * occupies: a conversation left open overnight reads "14h 3m", not "843m 12s".
 * Seconds are kept only while they help -- under ten minutes -- because past
 * that they are a tail that grows forever on a figure nobody reads to the
 * second, and "1222m 22s" is a number the reader has to divide before it means
 * anything.
 */

const SECONDS_PER_MINUTE = 60;
const SECONDS_PER_HOUR = 60 * SECONDS_PER_MINUTE;
const SECONDS_PER_DAY = 24 * SECONDS_PER_HOUR;
/**
 * A month is taken as thirty days.
 *
 * These are elapsed spans read at a glance, not calendar arithmetic: the exact
 * length of the month a span crosses is not something the reader is asking
 * about, and a figure that jumps by a day depending on which months it covers
 * would be more confusing than one that does not.
 */
const SECONDS_PER_MONTH = 30 * SECONDS_PER_DAY;

/** Up to this span, seconds are shown beside the minutes. Past it they are
    dropped, so the figure stops reeling off a growing tail. */
const SECOND_DETAIL_LIMIT = 10 * SECONDS_PER_MINUTE;

/** A duration, in the largest unit that still reads naturally.
 *
 * Coarser as it grows: minutes carry seconds, hours carry minutes, days carry
 * hours, and months and years carry the unit below and no further -- a
 * six-month span does not need the days it has accumulated. */
export function formatDuration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "—";
  const whole = Math.floor(seconds);

  if (whole < SECONDS_PER_MINUTE) return `${seconds.toFixed(1)}s`;

  if (whole < SECONDS_PER_HOUR) {
    const minutes = Math.floor(whole / SECONDS_PER_MINUTE);
    return whole < SECOND_DETAIL_LIMIT
      ? `${minutes}m ${whole % SECONDS_PER_MINUTE}s`
      : `${minutes}m`;
  }

  if (whole < SECONDS_PER_DAY) {
    const hours = Math.floor(whole / SECONDS_PER_HOUR);
    const minutes = Math.floor((whole % SECONDS_PER_HOUR) / SECONDS_PER_MINUTE);
    return minutes > 0 ? `${hours}h ${minutes}m` : `${hours}h`;
  }

  if (whole < SECONDS_PER_MONTH) {
    const days = Math.floor(whole / SECONDS_PER_DAY);
    const hours = Math.floor((whole % SECONDS_PER_DAY) / SECONDS_PER_HOUR);
    return hours > 0 ? `${days}d ${hours}h` : `${days}d`;
  }

  const months = Math.floor(whole / SECONDS_PER_MONTH);
  if (months < 12) {
    const days = Math.floor((whole % SECONDS_PER_MONTH) / SECONDS_PER_DAY);
    return days > 0 ? `${months}mo ${days}d` : `${months}mo`;
  }

  const years = Math.floor(months / 12);
  const remainder = months % 12;
  return remainder > 0 ? `${years}y ${remainder}mo` : `${years}y`;
}
