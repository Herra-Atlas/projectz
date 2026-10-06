import { useMemo, useState } from "react";

/** Which way a column is ordered. `null` until the column has been clicked. */
export type SortDirection = "asc" | "desc";

export type SortState<K extends string> = { column: K; direction: SortDirection } | null;

/**
 * Compares two rows by one column, in whichever direction is asked for.
 *
 * Numbers compare as numbers rather than as the strings the table draws, so
 * `9.1K` sorts below `48.2K` instead of above it. Ties fall back to the rows'
 * original order, which keeps a re-sort from shuffling rows that are genuinely
 * equal -- clicking the same column twice has to reverse the order, and a
 * comparator that reports "equal" for equal keys makes that reversal
 * unpredictable.
 */
function compareBy<T>(a: T, b: T, value: (row: T) => number, direction: SortDirection) {
  const left = value(a);
  const right = value(b);
  if (left === right) return 0;
  return direction === "asc" ? left - right : right - left;
}

/**
 * Click-to-sort for a table's numeric columns.
 *
 * `columns` maps a column id to the value it sorts on, so a table describes what
 * its columns mean rather than writing a comparator per header. The sort is
 * applied to a copy: `rows` is a prop holding the report, and sorting it in
 * place would reorder the caller's array.
 *
 * The direction starts at `default` for the column that is already ordered --
 * Models reads busiest-first and Conversations newest-first, which is what both
 * arrived as. So the first click on an *unordered* column shows the larger
 * values, which is the one a reader wants from a usage table, while the first
 * click on the already-sorted column reverses it. That keeps "press once for
 * most, again for least" true everywhere without a special case per column.
 */
export function useTableSort<T, K extends string>(
  rows: T[],
  columns: Record<K, (row: T) => number>,
  initial: SortState<K>,
) {
  const [sort, setSort] = useState<SortState<K>>(initial);
  const sorted = useMemo(() => {
    if (!sort) return rows;
    const value = columns[sort.column];
    // `map` to the index rather than sorting a copy alone, so the stable
    // tie-break below is available to hand equal rows to the comparator.
    return rows
      .map((row, index) => ({ row, index }))
      .sort((left, right) => {
        const byColumn = compareBy(left.row, right.row, value, sort.direction);
        return byColumn !== 0 ? byColumn : left.index - right.index;
      })
      .map((entry) => entry.row);
  }, [rows, columns, sort]);

  /** Clicking the sorted column reverses it; any other column starts at the default. */
  const toggle = (column: K) => {
    setSort((current) => {
      if (!current || current.column !== column) {
        return { column, direction: initial?.column === column ? "asc" : "desc" };
      }
      return { column, direction: current.direction === "desc" ? "asc" : "desc" };
    });
  };

  return {
    rows: sorted,
    sort,
    toggle,
    /** Direction for one column, so its header can draw the arrow. */
    directionFor: (column: K): SortDirection | null =>
      sort?.column === column ? sort.direction : null,
  };
}