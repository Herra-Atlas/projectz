import { parentOf } from "./useFileTree";

/**
 * Where a link inside a previewed file points, in the terms the Files panel
 * already speaks: a path relative to the open root, forward slashes.
 *
 * `null` means "not a file in this tree" -- an external URL, an anchor, an
 * absolute path, or a `..` that climbs above the root. The caller hands those to
 * the panel's browser instead, so a link is never silently dead.
 *
 * # Why the resolution happens here and not in Rust
 *
 * The path is relative to the *file*, and the file is relative to whichever root
 * the panel is showing (the workspace or a skill's folder). The backend resolves
 * a path against that root and deliberately refuses any `..` (`resolve_within`),
 * so walking the segments is the frontend's job: it is the only side that knows
 * what the link is written against. The backend still has the last word, because
 * the resolved path goes back through the same containment check as any other
 * panel read.
 *
 * # What is a segment
 *
 * Empty segments and `.` are skipped, `..` pops, and anything else pushes. A
 * leading `/` starts from the root rather than from the file's folder. A
 * backslash is a separator too, because a Markdown file written on Windows
 * occasionally carries one.
 */
export function resolveLinkPath(fromFile: string, href: string): string | null {
  const raw = href.trim();
  if (!raw) return null;
  // An in-document anchor has nowhere to go: the preview is not scroll-targeted
  // by id, and sending `#fragment` to the browser would be a search for the word.
  if (raw.startsWith("#")) return null;
  // A scheme (`https:`, `mailto:`, `file:`, ...) or a protocol-relative URL is
  // an address, not a file in this tree.
  if (/^[A-Za-z][A-Za-z0-9+.-]*:/.test(raw) || raw.startsWith("//")) return null;
  // A Windows absolute path would be refused by the backend anyway; refusing it
  // here says so before a round trip.
  if (/^[A-Za-z]:[\\/]/.test(raw)) return null;

  // A query orders a server, and a fragment names a place inside a page; neither
  // is part of the file name.
  const path = raw.split("#")[0].split("?")[0].replace(/\\/g, "/");
  if (!path) return null;

  const rooted = path.startsWith("/");
  const segments = rooted ? [] : parentOf(fromFile).split("/").filter(Boolean);

  for (const segment of path.split("/")) {
    if (!segment || segment === ".") continue;
    if (segment === "..") {
      // Above the root is not "the root": it is another tree the panel has no
      // business opening, and the backend would refuse the read.
      if (segments.length === 0) return null;
      segments.pop();
      continue;
    }
    segments.push(decodeSegment(segment));
  }

  // The root itself is not a file, so there is nothing to preview.
  if (segments.length === 0) return null;
  return segments.join("/");
}

/**
 * Whether an href is an address rather than a path in this tree.
 *
 * Split out because the two failures are not the same thing: a `https:` link
 * belongs in the browser, while a path that climbs out of the root belongs
 * nowhere -- sending it there would turn a broken link into a web search for a
 * file name.
 */
export function isExternalLink(href: string): boolean {
  const raw = href.trim();
  return /^[A-Za-z][A-Za-z0-9+.-]*:/.test(raw) || raw.startsWith("//");
}

/**
 * A percent-encoded segment as the name it stands for.
 *
 * A malformed escape is kept as written rather than dropped: a file literally
 * named `100%` is a file, and refusing it would turn a working link into
 * nothing for the sake of one bad character.
 */
function decodeSegment(segment: string): string {
  try {
    return decodeURIComponent(segment);
  } catch {
    return segment;
  }
}
