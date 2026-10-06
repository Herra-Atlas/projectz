/**
 * What one reply changed on disk.
 *
 * The activity panel records a write as an ordinary tool step: a label, a path,
 * and a diff. That is enough to audit one call, but a reply that edited nine
 * files leaves the reader counting rows to answer the only question they have:
 * what changed, and by how much. This aggregates the writes in one reply into
 * the single figure the header can state, and the per-file list under it.
 *
 * **Derived from the stored diff, never from a second count.** `ToolDiff` is
 * what the backend produced while it still held both versions of the file, so
 * the added and removed figures here are the same ones the diff view draws. A
 * count taken anywhere else would be a second implementation of a fact that
 * already exists, and the two would drift the first time a diff is capped.
 *
 * Only a successful write counts. A refused or failed call did not change
 * anything, and reporting its intended lines as changes would be a lie about
 * the filesystem — so a failed write is excluded entirely rather than counted at
 * zero.
 */

import type { ActivityStep, ToolDiff } from "./types";

/** One file's worth of change. */
export type FileChange = {
  /**
   * The path, as the tool reported it.
   *
   * Kept verbatim rather than shortened to a basename: two files of the same
   * name in different folders are the case where the basename hides the answer,
   * and the panel already truncates in the middle when a path is too long.
   */
  path: string;
  added: number;
  removed: number;
  /**
   * True when this file was created rather than edited.
   *
   * Read from the backend's own flag rather than inferred from the counts: a full
   * rewrite of an existing file has the same shape as a new one — every line
   * added, nothing removed — so the two are indistinguishable from a diff alone.
   */
  created: boolean;
};

const WRITE_TOOLS = new Set(["write_file", "edit_file", "edit_lines"]);

/**
 * Every file this reply wrote, in the order they were written.
 *
 * A file written twice appears once with the two diffs added together, which is
 * the figure a reader wants: "I changed 40 lines in this file", not "twice".
 * Ordering is first-write order rather than the order of the calls, so a file
 * edited early and again late sits where it was first touched.
 */
export function collectFileChanges(steps: ActivityStep[]): FileChange[] {
  const changes = new Map<string, FileChange>();

  for (const step of steps) {
    if (step.kind !== "tool") continue;
    // A diff is the marker that this call actually wrote. A `run_terminal` that
    // happened to touch a file carries no diff and so is not claimed here — the
    // backend cannot know what a command did, and a summary that guessed would
    // be worse than one that says nothing.
    if (!step.diff || step.failed) continue;
    if (!WRITE_TOOLS.has(step.tool)) continue;

    const path = step.detail;
    if (!path) continue;

    const existing = changes.get(path);
    const { added, removed } = countLines(step.diff);
    if (existing) {
      existing.added += added;
      existing.removed += removed;
      // A second write to a created file does not un-create it.
      continue;
    }
    changes.set(path, {
      path,
      added,
      removed,
      created: step.created === true,
    });
  }

  return [...changes.values()];
}

/**
 * Lines added and removed, read out of the diff's own kinds.
 *
 * Counts `context` as neither, because a context line is present before and
 * after: it is the run of unchanged lines the changes sit in, not part of the
 * change. `hidden` is deliberately ignored — it is lines the backend left out
 * of the diff to keep it small, so counting them would be inventing a figure.
 * The panel says a diff was capped where it draws one; this figure is what is
 * actually shown, which is the honest number.
 */
function countLines(diff: ToolDiff): { added: number; removed: number } {
  let added = 0;
  let removed = 0;
  for (const line of diff.lines) {
    if (line.kind === "add") added += 1;
    else if (line.kind === "remove") removed += 1;
  }
  return { added, removed };
}

/**
 * The totals for the header.
 *
 * Only additions and removals are summed, never a net figure. A net line count
 * is the one number that cannot answer either question worth asking: 546 added
 * and 324 removed is a large, well-understood change, while "+222" reads as a
 * tidy little edit and is the sum of two opposite things.
 */
export function totalChanges(changes: FileChange[]): { added: number; removed: number } {
  return changes.reduce(
    (total, change) => ({ added: total.added + change.added, removed: total.removed + change.removed }),
    { added: 0, removed: 0 },
  );
}

/**
 * A change worth showing at all.
 *
 * A write whose diff held nothing but context lines changed no lines, which
 * happens when a tool rewrote a file with identical contents. Reporting
 * "changed 1 file, +0 −0" is worse than reporting nothing: it is a row the
 * reader has to open to learn is empty.
 *
 * A created file counts even at zero, because creating an empty file is a real
 * change to the filesystem and the user asked for it.
 */
export function hasChanges(changes: FileChange[]): boolean {
  return changes.some((change) => change.created || change.added > 0 || change.removed > 0);
}

/** The file's own name, which is the part a reader scans for. */
export function fileName(path: string): string {
  const slash = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return slash === -1 ? path : path.slice(slash + 1);
}

/**
 * A count with its sign, for a figure that is always one or the other.
 *
 * `kind` is passed rather than a signed number because every caller has two
 * separate counts and no sign to infer from: a removal is a positive number that
 * happens to be a removal. Formatting on the sign alone would render a removal
 * as `+30`, in the colour reserved for an addition -- which is the one mistake
 * this figure cannot make, because a reader takes the colour as the meaning.
 *
 * A zero is rendered as `0` rather than `+0` or `−0`: callers omit the figure
 * entirely when there is nothing to say, so a zero here is not a count of no
 * changes but the absence of one side of the pair.
 */
export function signedCount(count: number, kind: "added" | "removed"): string {
  if (count === 0) return "0";
  return kind === "added" ? `+${count}` : `−${count}`;
}