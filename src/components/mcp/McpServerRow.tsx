import { Pencil, Trash2 } from "lucide-react";
import { HoldToDelete } from "../HoldToDelete";

import type { McpServerView } from "../../features/mcp/types";
import { commandLine } from "../../features/mcp/draft";
import { Toggle } from "../SettingsSection";
import McpServerIcon from "./McpServerIcon";

/**
 * One configured MCP server, drawn the way a provider is.
 *
 * The icon on the left is the disclosure -- the same control a provider row uses,
 * fetched and cached the same way -- and it carries the middle's name and
 * one-line summary while the actions sit on the right. Two screens holding the
 * same kind of thing should be read the same way.
 */
export default function McpServerRow({
  server,
  expanded,
  busy,
  onExpand,
  onToggle,
  onEdit,
  onDelete,
}: {
  server: McpServerView;
  expanded: boolean;
  busy: boolean;
  onExpand: () => void;
  onToggle: () => void;
  onEdit: () => void;
  onDelete: () => void;
}) {
  const command = commandLine(server);
  const toolCount = `${server.tools.length} ${server.tools.length === 1 ? "tool" : "tools"}`;

  return (
    <article>
      <div className="flex min-h-[68px] items-center gap-2 px-4 py-3">
        <McpServerIcon
          command={command}
          name={server.name || server.command}
          expanded={expanded}
          onToggle={onExpand}
        />

        <div className="min-w-0 flex-1">
          <h3 className="truncate text-sm font-medium">{server.name || server.command}</h3>
          {/* What it is, then how it went. A failure replaces the summary rather
              than sitting beside it, because a row that is not answering has
              nothing else worth saying on that line. */}
          {server.error ? (
            <p className="truncate text-xs text-[var(--danger)]" title={server.error}>
              {server.error}
            </p>
          ) : (
            <p className="truncate text-xs text-[var(--quiet)]" title={command}>
              {command}
              {server.enabled ? ` · ${toolCount}` : " · switched off"}
            </p>
          )}
        </div>

        <Toggle
          checked={server.enabled}
          onChange={onToggle}
          label={server.enabled ? `Disable ${server.name}` : `Enable ${server.name}`}
        />
        <button
          type="button"
          onClick={onEdit}
          className="grid size-9 shrink-0 place-items-center rounded-md text-[var(--muted)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
          aria-label={`Edit ${server.name}`}
          title="Edit server"
        >
          <Pencil size={16} />
        </button>
        <HoldToDelete
          onConfirm={onDelete}
          disabled={busy}
          label={`Remove ${server.name}`}
          title="Remove server"
          className="grid size-9 shrink-0 place-items-center rounded-md text-[var(--quiet)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--danger)] disabled:opacity-40 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
        >
          <Trash2 size={16} />
        </HoldToDelete>
      </div>

      {/* The tools, behind the same disclosure a provider keeps its models
          behind. The model-facing name is what a transcript will show, so it
          leads and the description follows. */}
      {expanded && (
        <div className="mb-3 ml-[2.75rem] max-h-64 overflow-y-auto border-l border-[var(--line)] py-1 pl-3">
          {server.tools.length === 0 ? (
            <p className="py-2 pr-2 text-xs text-[var(--quiet)]">
              {server.enabled
                ? "No tools listed. Refresh to reach the server."
                : "This server is switched off."}
            </p>
          ) : (
            <div className="divide-y divide-[var(--line)]">
              {server.tools.map((tool) => (
                <div key={tool.name} className="py-2 pr-2">
                  <p
                    className="truncate font-mono text-[12px] text-[var(--muted)]"
                    title={tool.name}
                  >
                    {tool.name}
                  </p>
                  {tool.description && (
                    <p
                      className="mt-0.5 line-clamp-2 text-[11px] leading-4 text-[var(--quiet)]"
                      title={tool.description}
                    >
                      {tool.description}
                    </p>
                  )}
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </article>
  );
}
