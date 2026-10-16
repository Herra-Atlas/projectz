/**
 * Reading and rewriting the workspace paths a user pastes.
 *
 * A pasted absolute path is long, and inside the workspace it says more than it
 * needs to: `F:\code\site\web\blackjack` is `web/blackjack` to anyone who knows
 * which folder is open. Two shapes of one fact, so both are derived here from a
 * single scan rather than three patterns that can disagree about where a path
 * ends.
 *
 * Only paths under the given root are touched. A path to somewhere else is not
 * the workspace's to shorten, and one that no longer sits under the root --
 * because the folder was closed, or another was opened -- is left exactly as it
 * was written rather than being shortened against the wrong prefix.
 */

/** A run of text, or a workspace path and the absolute form it came from. */
export type PathSegment = {
  text: string;
  /** Present only on a path segment: the full path, for the tooltip. */
  path?: string;
};

/** One path found in a body of text. */
type Occurrence = {
  /** Where it starts in the text. */
  start: number;
  /** Where it ends, after the path and before any punctuation that followed. */
  end: number;
  /** The absolute path as written, minus trailing punctuation. */
  absolute: string;
  /** The same path relative to the root, slash-separated. */
  relative: string;
};

/** Stops a path: whitespace, or a delimiter that never belongs in one. */
const PATH_CHAR = /[^\s"'`<>|]/;

/**
 * Punctuation a sentence put *after* a path rather than in it.
 *
 * Without this, "look at `…\web\blackjack.`" would keep the full stop and the
 * shortened form would be `web/blackjack.` -- and the send would put the stop
 * back inside the path it expands.
 */
const TRAILING_PUNCTUATION = /[.,;:!?)\]]+$/;

/** The root with any trailing separator removed, or `""` when that leaves nothing. */
function rootPrefix(root: string): string {
  return root.replace(/[\\/]+$/, "");
}

/**
 * The root as a pattern, tolerant of either separator and of case.
 *
 * Built character by character rather than by escaping the whole string and then
 * substituting separators: escaping first turns each `\` into `\\`, and a later
 * pass cannot tell a separator from an escaped one.
 */
function rootPattern(root: string): RegExp {
  const source = root
    .split("")
    .map((char) =>
      char === "\\" || char === "/"
        ? "[\\\\/]"
        : char.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"),
    )
    .join("");
  return new RegExp(source, "gi");
}

/** Every path in `text` that sits under `root`, in the order they appear. */
function findOccurrences(text: string, root: string): Occurrence[] {
  const prefix = rootPrefix(root);
  if (prefix.length === 0) return [];

  const found: Occurrence[] = [];
  for (const match of text.matchAll(rootPattern(prefix))) {
    const start = match.index;
    const afterRoot = start + match[0].length;

    // The root has to be followed by a separator or by nothing. `…\paskaa\testing`
    // spells the root up to `test` and then carries on, and trimming from there
    // would shorten a folder that is not the workspace to `ing/…`.
    if (afterRoot < text.length && text[afterRoot] !== "\\" && text[afterRoot] !== "/") continue;

    let end = afterRoot;
    while (end < text.length && PATH_CHAR.test(text[end])) end += 1;

    const run = text.slice(afterRoot, end).replace(TRAILING_PUNCTUATION, "");
    // The root alone names the folder, not a path within it, so there is nothing
    // to shorten it to.
    const relative = run.replace(/^[\\/]+/, "").replace(/[\\/]+/g, "/");
    if (relative.length === 0) continue;

    found.push({
      start,
      end: afterRoot + run.length,
      absolute: text.slice(start, afterRoot + run.length),
      relative,
    });
  }
  return found;
}

/** Splits text into runs, marking every workspace path it contains. */
export function splitWorkspacePaths(
  text: string,
  root: string | null | undefined,
): PathSegment[] {
  if (!root) return [{ text }];
  const found = findOccurrences(text, root);
  if (found.length === 0) return [{ text }];

  const segments: PathSegment[] = [];
  let cursor = 0;
  for (const occurrence of found) {
    if (occurrence.start > cursor) segments.push({ text: text.slice(cursor, occurrence.start) });
    segments.push({ text: occurrence.relative, path: occurrence.absolute });
    cursor = occurrence.end;
  }
  if (cursor < text.length) segments.push({ text: text.slice(cursor) });
  return segments;
}

/**
 * Shortens every path under `root`, returning the new text and what was
 * shortened so the send can put it back.
 */
export function trimWorkspacePaths(
  text: string,
  root: string | null | undefined,
): { text: string; trimmed: Map<string, string> } {
  const trimmed = new Map<string, string>();
  if (!root) return { text, trimmed };
  const found = findOccurrences(text, root);
  if (found.length === 0) return { text, trimmed };

  let result = "";
  let cursor = 0;
  for (const occurrence of found) {
    result += text.slice(cursor, occurrence.start) + occurrence.relative;
    trimmed.set(occurrence.relative, occurrence.absolute);
    cursor = occurrence.end;
  }
  return { text: result + text.slice(cursor), trimmed };
}

/**
 * Puts the full paths back.
 *
 * The composer shows the shorthand, but the message stores and sends what the
 * user actually pasted: the shorthand is a way to keep the field readable, not a
 * new syntax for a path, so nothing downstream has to know it exists.
 */
export function expandWorkspacePaths(text: string, trimmed: Map<string, string>): string {
  let result = text;
  for (const [relative, absolute] of trimmed) {
    if (relative !== absolute) result = result.split(relative).join(absolute);
  }
  return result;
}
