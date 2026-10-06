import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ChevronLeft, ChevronRight, Database, RefreshCw } from "lucide-react";

type DatabasePageProps = { refreshKey: number };
type TablePage = { rows: Record<string, unknown>[]; total: number };
const PAGE_SIZE = 20;

export default function DatabasePage({ refreshKey }: DatabasePageProps) {
  const [tables, setTables] = useState<string[]>([]);
  const [selected, setSelected] = useState("");
  const [rows, setRows] = useState<Record<string, unknown>[]>([]);
  const [total, setTotal] = useState(0);
  const [page, setPage] = useState(0);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [reload, setReload] = useState(0);

  useEffect(() => {
    let mounted = true;
    invoke<string[]>("database_tables").then((items) => {
      if (!mounted) return;
      setTables(items);
      setSelected((current) => current && items.includes(current) ? current : items[0] ?? "");
    }).catch((reason) => { if (mounted) setError(String(reason)); });
    return () => { mounted = false; };
  }, [refreshKey, reload]);

  useEffect(() => {
    if (!selected) return;
    let mounted = true;
    setLoading(true);
    setRows([]);
    invoke<TablePage>("database_table_rows", { table: selected, offset: page * PAGE_SIZE, limit: PAGE_SIZE }).then((result) => {
      if (mounted) { setRows(result.rows); setTotal(result.total); setError(""); }
    }).catch((reason) => { if (mounted) setError(String(reason)); }).finally(() => { if (mounted) setLoading(false); });
    return () => { mounted = false; };
  }, [selected, page, refreshKey, reload]);

  const columns = rows.length ? Object.keys(rows[0]) : [];
  const pageCount = Math.max(1, Math.ceil(total / PAGE_SIZE));

  return <main className="flex min-h-0 flex-1 flex-col">
    <header className="flex min-h-[68px] items-center justify-between border-b border-[var(--line)] px-5 sm:px-7">
      <div><h1 className="text-sm font-semibold">Database</h1><p className="text-xs text-[var(--quiet)]">Read-only view</p></div>
      <button type="button" onClick={() => setReload((value) => value + 1)} className="grid size-9 place-items-center rounded-md text-[var(--muted)] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]" aria-label="Refresh database view"><RefreshCw size={15} /></button>
    </header>
    <div className="flex min-h-0 flex-1">
      <nav aria-label="Database tables" className="w-44 shrink-0 overflow-y-auto border-r border-[var(--line)] bg-[var(--rail)] p-2 sm:w-56">
        <p className="px-2 py-2 text-[10px] font-medium uppercase tracking-wide text-[var(--quiet)]">Tables</p>
        {tables.map((table) => <button key={table} type="button" onClick={() => { setSelected(table); setPage(0); }} className={`flex min-h-9 w-full items-center gap-2 rounded-md px-2.5 text-left text-sm ${selected === table ? "bg-[var(--raised)] text-[var(--text)]" : "text-[var(--muted)] hover:bg-[var(--raised)]"}`}><Database size={14} />{table}</button>)}
      </nav>
      <section className="flex min-w-0 flex-1 flex-col" aria-label={`${selected || "Database"} table data`}>
        {error ? <p role="alert" className="p-5 text-sm text-[var(--danger)]">{error}</p> : !selected ? <p className="p-5 text-sm text-[var(--muted)]">No database tables found.</p> : <>
          {/* The scrolling region and the pager are siblings, so the pager stays
              put at the bottom instead of sitting below the last row. Only the
              table scrolls; the footer never leaves the viewport. */}
          <div className="min-h-0 flex-1 overflow-auto">
            {rows.length === 0 && !loading ? <p className="px-5 py-5 text-sm text-[var(--muted)]">This table is empty.</p> : <table className="w-full border-collapse text-left text-xs">
              <thead className="sticky top-0 z-10 bg-[var(--rail)]"><tr>{columns.map((column) => <th key={column} className="max-w-72 border-b border-r border-[var(--line)] px-3 py-2.5 font-medium text-[var(--muted)]">{column}</th>)}</tr></thead>
              <tbody>{rows.map((row, index) => <tr key={String(row.id ?? index)} className="align-top even:bg-[color-mix(in_srgb,var(--raised)_28%,transparent)]">{columns.map((column) => {
                // A cell holds one value and clips it. Long text and JSON blobs
                // are ordinary in here, and letting a cell grow to fit them made
                // a single `metrics_json` row taller than the screen and pushed
                // every other column's text out of view with it.
                const raw = row[column];
                const text = raw === null ? "NULL" : typeof raw === "object" ? JSON.stringify(raw) : String(raw);
                // Scrollable rather than clipped outright: a long value stays
                // fully readable by scrolling its own cell, which is better than
                // hiding the tail behind a `title` only hover can reach. The
                // scrollbar is thin and only shows while scrolling, so a short
                // value looks exactly as it did before.
                return <td key={column} className="max-w-96 border-b border-r border-[var(--line)] px-3 py-2 text-[var(--text)]"><span className="block max-h-20 overflow-y-auto overflow-x-hidden whitespace-pre-wrap break-words [scrollbar-width:thin] [scrollbar-color:var(--line)_transparent] [&::-webkit-scrollbar]:w-1.5 [&::-webkit-scrollbar-track]:bg-transparent [&::-webkit-scrollbar-thumb]:rounded-full [&::-webkit-scrollbar-thumb]:bg-[var(--line)]" title={text}>{text}</span></td>;
              })}</tr>)}</tbody>
            </table>}
          </div>
          <footer className="flex min-h-12 shrink-0 items-center justify-between border-t border-[var(--line)] bg-[var(--panel)] px-4 text-xs text-[var(--quiet)]">
            <span>{loading ? "Loading rows…" : total ? `${page * PAGE_SIZE + 1}–${Math.min((page + 1) * PAGE_SIZE, total)} of ${total}` : "0 rows"}</span>
            <div className="flex items-center gap-2"><button type="button" onClick={() => setPage((current) => Math.max(0, current - 1))} disabled={page === 0 || loading} className="grid size-8 place-items-center rounded hover:bg-[var(--raised)] disabled:opacity-35" aria-label="Previous page"><ChevronLeft size={15} /></button><span>{page + 1} / {pageCount}</span><button type="button" onClick={() => setPage((current) => Math.min(pageCount - 1, current + 1))} disabled={page + 1 >= pageCount || loading} className="grid size-8 place-items-center rounded hover:bg-[var(--raised)] disabled:opacity-35" aria-label="Next page"><ChevronRight size={15} /></button></div>
          </footer>
        </>}
      </section>
    </div>
  </main>;
}
