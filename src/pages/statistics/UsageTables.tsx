import { ArrowDown, ArrowUp, ArrowUpDown } from "lucide-react";
import { formatTimestamp, formatTokens } from "../../features/chat/sessionStats";
import { displayModelName } from "../../features/models/modelName";
import { useTableSort, type SortDirection } from "../../features/statistics/tableSort";
import type { ModelUsage, SessionUsage } from "../../features/statistics/types";

/** A header that can be ordered. The name column is simply absent from this list
    and stays plain text; the value each key sorts on lives in the `columns` map
    passed to `useTableSort`, so this only has to name the header. */
type SortableColumn<K extends string> = { key: K; label: string };

/** Shared table shell so both breakdowns line up: same header, same row height,
    same right-aligned numeric column. A header listed in `sortable` is a button,
    because a column that can be ordered has to say which way it is ordered. */
function UsageTable<K extends string>({
  head,
  sortable,
  directionFor,
  onSort,
  children,
  caption,
}: {
  head: string[];
  sortable: SortableColumn<K>[];
  directionFor: (column: K) => SortDirection | null;
  onSort: (column: K) => void;
  children: React.ReactNode;
  caption: string;
}) {
  return (
    <div className="max-h-[280px] overflow-auto">
      <table className="w-full border-collapse text-left text-xs">
        <caption className="sr-only">{caption}</caption>
        <thead className="sticky top-0 bg-[var(--panel)]">
          <tr>
            {head.map((label, index) => {
              const column = sortable.find((entry) => entry.label === label);
              const direction = column ? directionFor(column.key) : null;
              return (
                <th
                  key={label}
                  scope="col"
                  aria-sort={direction === "asc" ? "ascending" : direction === "desc" ? "descending" : undefined}
                  className={`border-b border-[var(--line)] px-4 py-2 font-medium text-[var(--quiet)] ${index === 0 ? "" : "text-right"}`}
                >
                  {column ? (
                    <button
                      type="button"
                      onClick={() => onSort(column.key)}
                      className={`group inline-flex min-h-6 items-center gap-1 rounded transition-colors hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)] ${index === 0 ? "" : "ml-auto"}`}
                    >
                      {label}
                      <SortArrow direction={direction} />
                    </button>
                  ) : (
                    label
                  )}
                </th>
              );
            })}
          </tr>
        </thead>
        <tbody>{children}</tbody>
      </table>
    </div>
  );
}

/** The direction a column is ordered in, drawn beside its label. The unsorted mark is
    a separate pair of arrows rather than a dimmed up arrow, because "ordered by
    something else" is a different state from "ordered ascending", and reusing one
    glyph for both makes the active column's direction ambiguous. */
function SortArrow({ direction }: { direction: SortDirection | null }) {
  const className = "shrink-0 text-[var(--accent)]";
  if (direction === "asc") return <ArrowUp size={11} strokeWidth={2.5} className={className} />;
  if (direction === "desc") return <ArrowDown size={11} strokeWidth={2.5} className={className} />;
  return <ArrowUpDown size={11} strokeWidth={2} className="shrink-0 text-[var(--quiet)] opacity-0 transition-opacity group-hover:opacity-100" />;
}

/** The views the models panel pages through.
 *
 * `local` is its own view rather than one provider among several, because a
 * local reply carries no provider at all -- it is not a provider that happens to
 * be named blank, and grouping it with the remote ones would misreport where it
 * ran.
 */
export type ModelView = "all" | "local" | string;

/** One view's worth of rows. `label` names the view in the panel header. */
type ModelGroup = { label: string; models: ModelUsage[] };

/**
 * Builds the views: everything, then local, then one per provider.
 *
 * A provider is named by its own `name` rather than the raw `provider_id` stored
 * on the message, because that id is an internal key and a reader wants the name
 * they gave the provider in Settings. The name is resolved from the endpoint
 * list; an id with no matching endpoint falls back to the id itself rather than
 * being dropped, so usage from a removed provider is still accounted for.
 */
export function modelGroups(models: ModelUsage[], providerNames: Map<string, string>): ModelGroup[] {
  /** A local reply has no provider: the alias stands in for the model, so the
      provider column is empty and there is nothing to group it under. */
  const isLocal = (model: ModelUsage) => model.model_id === "projectz-local" || !model.provider_id;

  const local = models.filter(isLocal);
  const byProvider = new Map<string, ModelUsage[]>();
  for (const model of models.filter((entry) => !isLocal(entry))) {
    byProvider.set(model.provider_id, [...(byProvider.get(model.provider_id) ?? []), model]);
  }

  const groups: ModelGroup[] = [{ label: "All", models }];
  if (local.length > 0) groups.push({ label: "Local", models: local });
  for (const [id, rows] of [...byProvider.entries()].sort((a, b) => a[0].localeCompare(b[0]))) {
    groups.push({ label: providerNames.get(id) ?? id, models: rows });
  }
  return groups;
}

/** Every model that answered in the period, in whichever view is selected.
 *
 * Defaults to busiest-first, which is the order the report arrived in. The
 * numeric headers are buttons; clicking one orders by that figure and clicking
 * it again reverses. */
export function ModelTable({ group }: { group: ModelGroup }) {
  const columns = {
    replies: (row: ModelUsage) => row.replies,
    input: (row: ModelUsage) => row.prompt_tokens,
    output: (row: ModelUsage) => row.completion_tokens,
  } as const;
  const { rows, toggle, directionFor } = useTableSort<ModelUsage, keyof typeof columns>(group.models, columns, { column: "replies", direction: "desc" });
  if (group.models.length === 0) {
    return <p className="px-4 py-10 text-center text-sm text-[var(--muted)]">No model recorded in this period.</p>;
  }
  return (
    <UsageTable<keyof typeof columns>
      head={["Model", "Replies", "Input", "Output"]}
      sortable={[
        { key: "replies", label: "Replies" },
        { key: "input", label: "Input" },
        { key: "output", label: "Output" },
      ]}
      directionFor={directionFor}
      onSort={toggle}
      caption="Models used in this period"
    >
      {rows.map((model) => (
        <tr key={`${model.provider_id}:${model.model_id}`} className="border-b border-[var(--line)] last:border-b-0">
          <th scope="row" className="max-w-0 px-4 py-2 font-normal">
            <span className="block truncate text-[var(--text)]" title={model.model_id}>{displayModelName(model.model_id)}</span>
            {/* The provider subline is dropped inside a per-provider view, where
                every row already says the same thing. */}
            {model.provider_id && group.label === "All" && <span className="block truncate text-[11px] text-[var(--quiet)]" title={model.provider_id}>{model.provider_id}</span>}
          </th>
          <td className="px-4 py-2 text-right tabular-nums text-[var(--muted)]">{model.replies.toLocaleString()}</td>
          <td className="px-4 py-2 text-right tabular-nums text-[var(--muted)]">{formatTokens(model.prompt_tokens)}</td>
          <td className="px-4 py-2 text-right tabular-nums text-[var(--text)]">{formatTokens(model.completion_tokens)}</td>
        </tr>
      ))}
    </UsageTable>
  );
}

/** Conversations started in the period, newest first.
 *
 * A title is clickable and opens the same read-only overview the sidebar's
 * session menu opens, so a row here is a way into the conversation rather than
 * a dead end. `onOpen` is absent when the full session is not loaded, in which
 * case the title renders as plain text instead of a control that does nothing.
 *
 * `Started` sorts on the timestamp itself rather than on the formatted string,
 * so an ISO-8601 date compares chronologically instead of alphabetically. */
export function SessionTable({
  sessions,
  onOpen,
}: {
  sessions: SessionUsage[];
  onOpen?: (id: string) => void;
}) {
  const columns = {
    messages: (row: SessionUsage) => row.message_count,
    tokens: (row: SessionUsage) => row.prompt_tokens + row.completion_tokens,
    started: (row: SessionUsage) => new Date(row.created_at).getTime(),
  } as const;
  const { rows, toggle, directionFor } = useTableSort<SessionUsage, keyof typeof columns>(sessions, columns, { column: "started", direction: "desc" });
  if (sessions.length === 0) {
    return <p className="px-4 py-10 text-center text-sm text-[var(--muted)]">No conversations in this period.</p>;
  }
  return (
    <UsageTable<keyof typeof columns>
      head={["Conversation", "Messages", "Tokens", "Started"]}
      sortable={[
        { key: "messages", label: "Messages" },
        { key: "tokens", label: "Tokens" },
        { key: "started", label: "Started" },
      ]}
      directionFor={directionFor}
      onSort={toggle}
      caption="Conversations in this period"
    >
      {rows.map((session) => {
        const total = session.prompt_tokens + session.completion_tokens;
        return (
          <tr key={session.id} className="border-b border-[var(--line)] last:border-b-0">
            <th scope="row" className="max-w-0 px-4 py-2 font-normal">
              {onOpen ? (
                <button
                  type="button"
                  onClick={() => onOpen(session.id)}
                  title={`Open overview for ${session.title}`}
                  className="block max-w-full truncate rounded text-left text-[var(--text)] underline-offset-2 hover:text-[var(--accent)] hover:underline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
                >
                  {session.title}
                </button>
              ) : (
                <span className="block truncate text-[var(--text)]" title={session.title}>{session.title}</span>
              )}
            </th>
            <td className="px-4 py-2 text-right tabular-nums text-[var(--muted)]">{session.message_count.toLocaleString()}</td>
            <td className="px-4 py-2 text-right tabular-nums text-[var(--text)]">{formatTokens(total)}</td>
            <td className="px-4 py-2 text-right text-[var(--quiet)]">{formatTimestamp(session.created_at)}</td>
          </tr>
        );
      })}
    </UsageTable>
  );
}
