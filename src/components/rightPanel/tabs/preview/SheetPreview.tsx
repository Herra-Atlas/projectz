import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import PreviewNote from "./PreviewNote";

/** One sheet as the backend reads it. */
type SheetGrid = {
  name: string;
  rows: string[][];
  /** True when the sheet was longer than the preview's limit. */
  truncated: boolean;
};

/** The column's name as a spreadsheet shows it: A, B, ... Z, AA. */
function columnLetters(index: number): string {
  let name = "";
  let value = index + 1;
  while (value > 0) {
    const remainder = (value - 1) % 26;
    name = String.fromCharCode(65 + remainder) + name;
    value = Math.floor((value - 1) / 26);
  }
  return name;
}

/**
 * A spreadsheet, as a grid.
 *
 * **Parsed in Rust, drawn here.** The reader that turns a `.xlsx` into rows is the
 * same one `read_file` uses, so a model reading a sheet and a person looking at it
 * cannot come to different conclusions about what it contains. Stringifying the
 * cells in the frontend would mean a second opinion about a skipped column, which
 * is exactly the mistake that shifts every value after a gap one cell to the left.
 *
 * **Column letters and row numbers, not the first row as a heading.** A grid is a
 * grid: the first row is data unless the writer was told otherwise, and dressing it
 * as a header would be the panel guessing. The letters are also what a reader uses
 * to find a cell in Excel, so they are the useful thing to freeze.
 */
export default function SheetPreview({ path }: { path: string }) {
  const [sheets, setSheets] = useState<SheetGrid[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [active, setActive] = useState(0);

  useEffect(() => {
    let alive = true;
    setSheets(null);
    setError(null);
    setActive(0);
    invoke<SheetGrid[]>("panel_sheet_grid", { relative: path })
      .then((loaded) => {
        if (alive) setSheets(loaded);
      })
      .catch((reason: unknown) => {
        if (alive) setError(String(reason));
      });
    return () => {
      alive = false;
    };
  }, [path]);

  if (error) return <PreviewNote tone="error">{error}</PreviewNote>;
  if (!sheets) return <PreviewNote>Reading…</PreviewNote>;

  const sheet = sheets[Math.min(active, sheets.length - 1)];
  if (!sheet || sheet.rows.length === 0) {
    return <PreviewNote>This workbook has no cells to show.</PreviewNote>;
  }

  const columns = sheet.rows.reduce((widest, row) => Math.max(widest, row.length), 0);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {sheets.length > 1 && (
        <div className="flex min-h-8 shrink-0 items-center gap-1 overflow-x-auto border-b border-[var(--line)] px-2">
          {sheets.map((entry, index) => (
            <button
              key={`${index}-${entry.name}`}
              type="button"
              onClick={() => setActive(index)}
              aria-pressed={index === active}
              className={`min-h-6 shrink-0 rounded px-2 text-[11px] transition-colors ${
                index === active
                  ? "bg-[var(--raised)] text-[var(--text)]"
                  : "text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]"
              }`}
            >
              {entry.name}
            </button>
          ))}
        </div>
      )}

      <div className="min-h-0 flex-1 overflow-auto">
        <table className="border-collapse text-[11px] tabular-nums">
          <thead>
            {/* Both axes are frozen: without the row numbers a scrolled sheet
                loses track of which line is which. */}
            <tr>
              <th className="sticky left-0 top-0 z-20 min-w-9 border border-[var(--line)] bg-[var(--raised)] px-1 py-0.5 text-[10px] font-normal text-[var(--quiet)]" />
              {Array.from({ length: columns }, (_, column) => (
                <th
                  key={column}
                  scope="col"
                  className="sticky top-0 z-10 min-w-20 border border-[var(--line)] bg-[var(--raised)] px-2 py-0.5 text-left text-[10px] font-normal text-[var(--quiet)]"
                >
                  {columnLetters(column)}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {sheet.rows.map((row, index) => (
              <tr key={index}>
                <th
                  scope="row"
                  className="sticky left-0 z-10 border border-[var(--line)] bg-[var(--raised)] px-1 py-0.5 text-right text-[10px] font-normal text-[var(--quiet)]"
                >
                  {index + 1}
                </th>
                {Array.from({ length: columns }, (_, column) => (
                  <td
                    key={column}
                    // Values are trimmed to one line: a cell holding a paragraph
                    // would otherwise set the row's height for the whole sheet.
                    className="max-w-72 truncate border border-[var(--line)] px-2 py-0.5 text-[var(--text)]"
                    title={row[column] ?? ""}
                  >
                    {row[column] ?? ""}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      {sheet.truncated && (
        <p className="shrink-0 border-t border-[var(--line)] px-3 py-1.5 text-[11px] text-[var(--quiet)]">
          Showing the first rows and columns. The file itself is complete.
        </p>
      )}
    </div>
  );
}
