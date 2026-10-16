import { useState, type ReactNode } from "react";
import { LoaderCircle } from "lucide-react";

import type { McpDraft } from "../../features/mcp/draft";
import HelpMark from "../HelpMark";
import { SettingsSection } from "../SettingsSection";

/** The hint under the key field, which explains both fields above it. */
const KEY_HINT =
  "A URL starting with http:// or https:// connects over HTTP. Anything else runs as a command. The key is handed to the command as MCP_API_KEY. Type NAME=value to use another variable.";

/**
 * One labelled field, in the two-column shape the rest of the app's forms use:
 * the name on the left where it can be scanned, the control on the right.
 *
 * The `?` carries the long explanation so the row stays one line tall. Only the
 * key's hint is written out, because it describes two fields at once and is the
 * one a user has to read to use the form at all.
 */
function Row({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: ReactNode;
}) {
  return (
    <div className="flex flex-col gap-2 px-4 py-3.5 sm:flex-row sm:items-start sm:gap-4">
      <div className="flex w-[150px] shrink-0 items-center gap-1.5 text-[13px] font-medium text-[var(--text)] sm:min-h-9">
        <span>{label}</span>
        {hint && <HelpMark hint={hint} />}
      </div>
      <div className="min-w-0 flex-1">{children}</div>
    </div>
  );
}

const field =
  "h-9 w-full rounded-md border border-[var(--line)] bg-[var(--rail)] px-3 text-[13px] text-[var(--text)] outline-none placeholder:text-[var(--quiet)] focus:border-[var(--accent)]";
const monoField = `${field} font-mono`;
const areaField =
  "w-full resize-y rounded-md border border-[var(--line)] bg-[var(--rail)] px-3 py-2 font-mono text-[13px] leading-6 text-[var(--text)] outline-none placeholder:text-[var(--quiet)] focus:border-[var(--accent)]";

/**
 * The add/edit page for one MCP server.
 *
 * **The command line is one field.** A program and its arguments are one thing a
 * user copies from a server's README, and splitting them across two boxes makes
 * them paste it into the wrong one. It is split on save, the way the shell that
 * runs it would split it.
 *
 * **Nothing is validated beyond "a command is required".** A server is an
 * arbitrary program with arbitrary arguments, so this screen has no way to know
 * what a correct line looks like -- the only real test is starting it, which is
 * what saving does. A wrong command reported as a form error would be this page
 * guessing; reported as what the program printed, it is the truth.
 */
export default function McpServerForm({
  draft,
  busy,
  onSave,
  onCancel,
  onDelete,
}: {
  draft: McpDraft;
  busy: boolean;
  onSave: (draft: McpDraft) => void;
  onCancel: () => void;
  onDelete: () => void;
}) {
  const [form, setForm] = useState(draft);
  const editing = draft.id !== "";

  const set = <K extends keyof McpDraft>(key: K, value: McpDraft[K]) =>
    setForm((current) => ({ ...current, [key]: value }));

  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        if (form.command.trim().length > 0) onSave(form);
      }}
    >
      <SettingsSection title="Servers">
        <Row label="Name" hint="Names the tools the model sees. Blank means the command stands in.">
          <input
            value={form.name}
            onChange={(event) => set("name", event.target.value)}
            placeholder="playwright"
            className={field}
          />
        </Row>
        <Row
          label="Command or URL"
          hint="The program to run, with its arguments, as it would be typed in a terminal — npx, node, uvx, an absolute path. A http:// or https:// address connects over HTTP instead."
        >
          <input
            value={form.command}
            onChange={(event) => set("command", event.target.value)}
            placeholder="npx -y @modelcontextprotocol/server-foo"
            required
            spellCheck={false}
            className={monoField}
          />
        </Row>
        <Row
          label="API key"
          hint="Sent as MCP_API_KEY to a command, or as a bearer token to a URL. NAME=value sets the variable yourself."
        >
          <input
            value={form.key}
            onChange={(event) => set("key", event.target.value)}
            placeholder="Optional"
            spellCheck={false}
            className={monoField}
          />
          <p className="mt-2 text-[12px] leading-5 text-[var(--quiet)]">{KEY_HINT}</p>
        </Row>
        <Row
          label="Environment"
          hint="Added to the app's own environment for this server only. One NAME=value per line."
        >
          <textarea
            value={form.env}
            onChange={(event) => set("env", event.target.value)}
            rows={3}
            spellCheck={false}
            placeholder="API_KEY=…"
            className={areaField}
          />
        </Row>
      </SettingsSection>

      <div className="mt-6 flex items-center gap-2">
        <button
          type="submit"
          disabled={busy || form.command.trim().length === 0}
          className="inline-flex min-h-9 items-center gap-2 rounded-lg bg-[var(--text)] px-3.5 text-[13px] font-medium text-[var(--page)] transition-colors hover:bg-[var(--accent)] hover:text-[var(--accent-ink)] disabled:cursor-not-allowed disabled:opacity-35 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
        >
          {busy && <LoaderCircle size={14} className="animate-spin" />}
          {editing ? "Save and connect" : "Add and connect"}
        </button>
        <button
          type="button"
          onClick={onCancel}
          className="min-h-9 rounded-lg px-3 text-[13px] text-[var(--muted)] transition-colors hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
        >
          Cancel
        </button>
        {editing && (
          <button
            type="button"
            onClick={onDelete}
            disabled={busy}
            className="ml-auto min-h-9 rounded-lg px-3 text-[13px] text-[var(--muted)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--danger)] disabled:opacity-40 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
          >
            Remove server
          </button>
        )}
      </div>
    </form>
  );
}
