/**
 * Sizes, as a settings row shows them.
 *
 * Its own module because two pages now show one: the local model's memory
 * estimate and the browser profile's footprint on disk. A second copy of the
 * rounding would be a second place for the two figures to disagree about what a
 * gigabyte is.
 */

/**
 * A byte count in the largest unit that keeps it readable.
 *
 * MB below a gigabyte and GB above it, because that is the range both callers
 * live in, and a figure in the wrong one ("1334 MB") is one a reader has to do
 * arithmetic on to compare with anything else on the page.
 */
export const formatBytes = (bytes: number) => {
  if (!bytes || bytes < 1) return "0 MB";
  const gb = bytes / 1024 ** 3;
  return gb >= 1 ? `${gb.toFixed(1)} GB` : `${(bytes / 1024 ** 2).toFixed(0)} MB`;
};
