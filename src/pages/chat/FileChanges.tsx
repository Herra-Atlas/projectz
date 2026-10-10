import { useState } from "react";
import { ChevronRight, LoaderCircle } from "lucide-react";
import ToolIcon from "../../components/ToolIcon";
import type { ActivityStep } from "../../features/chat/types";
import {
  collectFileChanges,
  fileName,
  hasChanges,
  isWriteTool,
  signedCount,
  totalChanges,
  type FileChange,
} from "../../features/chat/fileChanges";

type FileChangesProps = {
  steps: ActivityStep[];
  /** True while the reply these steps belong to is still running. */
  live: boolean;
  /**
   * Opens a file in the right panel.
   *
   * The paths here are already workspace-relative -- the tools refuse absolute
   * paths, so a write could not have reported one -- which is exactly the form
   * the panel's read command takes. That is why the whole thing is a path handed
   * across rather than an entry looked up in the loaded tree: the file may be in
   * a folder nobody expanded.
   */
  onOpenFile?: (path: string) => void;
};

/**
 * What a reply wrote, below the answer.
 *
 * **After the text rather than inside the panel.** The panel records what the
 * agent did as it did it -- one row per call, in order, which is what someone
 * opening it wants. This is the same fact stated once, and placing it there
 * meant a reader had to open the panel to find out whether anything was written
 * at all. It is a fact about the reply, so it reads after the reply.
 *
 * **Styled as the panel's own rows, but not inside it.** A bordered card in the
 * transcript drew attention to a summary more than to the answer above it, and
 * the row it grew out of was already saying the same thing. A row's type,
 * spacing, icon and caret, with no border and no background, is the same
 * information at the weight the surrounding text already uses.
 *
 * Absent entirely when the reply wrote nothing, so a reply that only read files
 * carries no row claiming otherwise.
 */
export default function FileChanges({ steps, live, onOpenFile }: FileChangesProps) {
  const [expanded, setExpanded] = useState(false);
  const changes = collectFileChanges(steps);
  if (!hasChanges(changes)) return null;

  const totals = totalChanges(changes);
  // A write still in flight carries no diff yet, so it contributes nothing here
  // and the row simply appears once the result lands. The spinner is the panel's
  // own convention for "this is happening", reused rather than invented.
  const writing = live && steps.some((step) => step.kind === "tool" && step.running && step.diff === undefined && isWriteTool(step.tool));

  return (
    <div className="mt-2 max-w-2xl text-[12px] leading-6 text-[var(--muted)]">
      <div className="flex items-center gap-1.5 py-0.5">
        <button
          type="button"
          onClick={() => setExpanded((value) => !value)}
          aria-expanded={expanded}
          className="flex min-h-7 min-w-0 flex-1 items-center gap-1.5 rounded text-left hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
        >
          {/* The write tool's own mark, so the row is recognisable as a write
              rather than as a summary invented by the app. */}
          <ToolIcon tool="write_file" />
          <span className="shrink-0 text-[var(--muted)]">{changes.length === 1 ? `Changed ${fileName(changes[0].path)}` : `Changed ${changes.length} files`}</span>
          {/* The totals, in the panel's `--quiet` rather than in the diff colours:
              the figures here sit among a dozen other rows, and a saturated
              green and red on every reply would outrank the answer they describe.
              The per-file rows below keep the colours, where they are the point. */}
          {(totals.added > 0 || totals.removed > 0) && (
            <span className="flex shrink-0 items-center gap-1.5 font-mono text-[11px] tabular-nums text-[var(--quiet)]">
              {totals.added > 0 && <span>{signedCount(totals.added, "added")}</span>}
              {totals.removed > 0 && <span>{signedCount(totals.removed, "removed")}</span>}
            </span>
          )}
          {writing && <LoaderCircle size={11} className="shrink-0 animate-spin text-[var(--accent)]" />}
          <ChevronRight size={12} aria-hidden className={`shrink-0 transition-transform ${expanded ? "rotate-90" : ""}`} />
        </button>
      </div>

      {expanded && (
        // Indented to the tool icon and ruled on the left, matching the panel's
        // own folded-run and thought disclosures, so this opens like everything
        // else in the list rather than as its own thing.
        <ul className="ml-[18px] space-y-0.5 border-l border-[var(--line)] pb-1 pl-3">
          {changes.map((change) => (
            <FileChangeRow key={change.path} change={change} onOpenFile={onOpenFile} />
          ))}
        </ul>
      )}
    </div>
  );
}

/** One file, with its own counts. */
function FileChangeRow({ change, onOpenFile }: { change: FileChange; onOpenFile?: (path: string) => void }) {
  return (
    <li className="flex min-h-7 min-w-0 items-center gap-1.5 text-[11px]">
      {/* The row is the button, not the name inside it. A target the size of one
          12px word is a target people miss, and the counts and path already make
          the row read as something clickable. Falls back to plain text when the
          panel is not there to open into, so a reply never carries a dead
          control. */}
      <button
        type="button"
        onClick={() => onOpenFile?.(change.path)}
        disabled={!onOpenFile}
        title={onOpenFile ? `Open ${change.path}` : undefined}
        className="flex min-h-7 min-w-0 flex-1 items-center gap-1.5 rounded text-left transition-colors hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)] disabled:pointer-events-none"
      >
        <ToolIcon tool="write_file" size={11} />
        <span className="shrink-0 text-[var(--muted)]">{fileName(change.path)}</span>
        {/* Coloured here, unlike the header: this row *is* about the change, so the
            figures are the content rather than a figure among several. */}
        {(change.added > 0 || change.removed > 0) && (
          <span className="flex shrink-0 items-center gap-1.5 font-mono text-[11px] tabular-nums">
            {change.added > 0 && <span className="text-[var(--added)]">{signedCount(change.added, "added")}</span>}
            {change.removed > 0 && <span className="text-[var(--removed)]">{signedCount(change.removed, "removed")}</span>}
          </span>
        )}
        {/* The path, truncated, as every other row in the panel shows it. A
            created file is marked, since a file the agent brought into existence
            is a different thing from one it edited -- and a full rewrite is not,
            because both look the same in a diff. */}
        <span className="min-w-0 truncate text-[var(--quiet)]">
          {change.created ? `created · ${change.path}` : change.path}
        </span>
      </button>
    </li>
  );
}