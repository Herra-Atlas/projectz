import { useState } from "react";
import { LoaderCircle, Plus, RefreshCw } from "lucide-react";

import type { Notify } from "../features/notifications/types";
import type { McpDraft } from "../features/mcp/draft";
import { NEW_DRAFT, draftFrom, serverFrom } from "../features/mcp/draft";
import { useMcpServers } from "../features/mcp/useMcpServers";
import McpServerForm from "./mcp/McpServerForm";
import McpServerRow from "./mcp/McpServerRow";
import { ANCHOR_ATTRIBUTE, FlashHighlight, SettingsPageTitle } from "./SettingsSection";

/**
 * Settings → MCP servers.
 *
 * An MCP server is a program or a URL that offers tools over the Model Context
 * Protocol: a filesystem, a database, a browser. Once one is added and switched
 * on, its tools are offered to the agent beside the built-in ones, named
 * `mcp__<server>__<tool>`.
 *
 * **Shaped like the Providers page**, on purpose: a list of rows with a
 * disclosure each, and adding one is a page of its own. Two screens that hold the
 * same kind of thing -- a configured external capability with a name, a set of
 * tools and a state -- should be read the same way rather than each inventing a
 * layout.
 *
 * **Saving connects before it closes.** A command line is the one thing here that
 * can be wrong in a way only the program can explain, and that explanation is
 * worth having while the fields are still open.
 */
export default function McpSettingsPage({ notify }: { notify: Notify }) {
  const { servers, loading, busy, refresh, save, remove } = useMcpServers();
  const [draft, setDraft] = useState<McpDraft | null>(null);
  const [expanded, setExpanded] = useState<string[]>([]);

  /** Runs one action, reporting a failure instead of throwing it at the shell. */
  const guarded = async (action: () => Promise<unknown>, failure: string) => {
    try {
      await action();
      return true;
    } catch (reason) {
      notify("error", `${failure}: ${String(reason)}`);
      return false;
    }
  };

  if (draft) {
    return (
      <McpServerForm
        draft={draft}
        busy={busy}
        onCancel={() => setDraft(null)}
        onSave={async (next) => {
          const server = serverFrom(next);
          const label = server.name || server.command;
          const saved = await guarded(() => save(server), `Could not save ${label}`);
          if (saved) {
            setDraft(null);
            notify("success", `${label} saved`);
          }
        }}
        onDelete={() => {
          const id = draft.id;
          void guarded(() => remove(id), "Could not remove the server").then((done) => {
            if (done) setDraft(null);
          });
        }}
      />
    );
  }

  return (
    <>
      <SettingsPageTitle>
        <h2 className="text-[17px] font-semibold tracking-tight" {...{ [ANCHOR_ATTRIBUTE]: "mcp.servers" }}>MCP servers</h2>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => void guarded(refresh, "Could not reach the servers")}
            disabled={busy}
            title="Start every enabled server and list its tools"
            className="inline-flex min-h-9 items-center gap-1.5 rounded-lg border border-[var(--line)] bg-[var(--panel)] px-3 text-[13px] font-medium text-[var(--text)] transition-colors hover:bg-[var(--raised)] disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
          >
            {busy ? <LoaderCircle size={15} className="animate-spin" /> : <RefreshCw size={15} />}
            Refresh
          </button>
          <button
            type="button"
            onClick={() => setDraft({ ...NEW_DRAFT })}
            className="inline-flex min-h-9 items-center gap-1.5 rounded-lg border border-[var(--line)] bg-[var(--panel)] px-3 text-[13px] font-medium text-[var(--text)] transition-colors hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
          >
            <FlashHighlight anchor="mcp.servers" />
            <Plus size={15} />
            Add server
          </button>
        </div>
      </SettingsPageTitle>

      {loading ? (
        <p className="flex items-center gap-2 text-[13px] text-[var(--muted)]">
          <LoaderCircle size={15} className="animate-spin" />
          Loading servers…
        </p>
      ) : servers.length === 0 ? (
        <div className="px-4 py-7">
          <p className="text-sm text-[var(--muted)]">No MCP servers yet.</p>
        </div>
      ) : (
        <div className="divide-y divide-[var(--line)]">
          {servers.map((server) => (
            <McpServerRow
              key={server.id}
              server={server}
              expanded={expanded.includes(server.id)}
              busy={busy}
              onExpand={() =>
                setExpanded((current) =>
                  current.includes(server.id)
                    ? current.filter((id) => id !== server.id)
                    : [...current, server.id],
                )
              }
              onToggle={() => {
                // Toggling goes through save, so the switch and the stored row
                // cannot disagree: one write, then one attempt to reach it.
                const next = { ...server, enabled: !server.enabled };
                void guarded(
                  () => save(next),
                  `Could not ${next.enabled ? "enable" : "disable"} ${server.name}`,
                );
              }}
              onEdit={() => setDraft(draftFrom(server))}
              onDelete={() => {
                void guarded(() => remove(server.id), `Could not remove ${server.name}`);
              }}
            />
          ))}
        </div>
      )}
    </>
  );
}
