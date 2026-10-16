/**
 * What counts as a link, in one place.
 *
 * Two callers need the same answer -- the selection bar (is this highlight a URL
 * I may offer to open?) and a code block (which runs of this text are URLs?) --
 * so the rules live here rather than being written twice and drifting.
 *
 * Accepted shapes:
 *
 *   - an explicit `http(s)://` address,
 *   - `localhost` or `127.0.0.1`, optionally with a port -- the address a dev
 *     server prints, and the one most worth opening beside the chat,
 *   - a bare `host.tld`, optionally with a path -- `example.com`, `github.com/x`.
 *
 * A bare host cannot be told from a filename by shape alone: `example.com` and
 * `App.css` are the same three tokens. So the extension wins the tie -- a run
 * whose last label is a known file extension is a file, not a page, and nothing
 * is offered for it. The reverse mistake (offering to open a file) is the one
 * that can be acted on wrongly; a missed domain merely never becomes a link.
 */

/** `localhost` / `127.0.0.1`, optionally with a port and path. */
const LOCAL_HOST = /^(localhost|127\.0\.0\.1)(:\d+)?([/?#][^\s]*)?$/i;

/** Any other bare host: at least one dot, no userinfo, optional path/query. */
const BARE_HOST = /^[^\s/?#@]+\.[^\s/?#@]+([/?#][^\s]*)?$/;

/**
 * Candidate URLs as they are written in running text or code.
 *
 * Three alternatives, longest shape first: a scheme URL, a loopback address
 * (which carries no dot when written as `localhost`), and a bare `host.tld`. A
 * match is a *span*, not a verdict -- `asNavigableUrl` still has the last word --
 * because this has to be loose enough to find an address inside `href="..."`.
 * Quotes and angle brackets end a match so the markup around a link is not
 * swallowed along with it, and the lookbehind keeps a host from being found
 * *inside* a word -- without it the `b.com` of `a@b.com` reads as a link.
 */
const CANDIDATE = /https?:\/\/[^\s<>"'`]+|(?<![\w@])(?:localhost|127\.0\.0\.1)(?::\d+)?(?:\/[^\s<>"'`]*)?|(?<![\w@])[a-z0-9][a-z0-9-]*(?:\.[a-z0-9-]+)+(?::\d+)?(?:\/[^\s<>"'`]*)?/gi;

/** Punctuation that trails a URL in a sentence rather than belonging to it. */
const TRAILING_PUNCTUATION = /[.,;:!?)\]}>]+$/;

/**
 * Last labels that mean a file rather than a domain.
 *
 * Broad on purpose: a bare host is accepted by default, so every entry here is
 * one the shape alone would otherwise wave through. Several are also real TLDs --
 * `.rs`, `.md`, `.py`, `.sh`, `.pl`, `.so`, `.cc` -- which is exactly why this
 * list exists rather than one of them being trusted as an address.
 */
const FILE_EXTENSIONS = new Set([
  "ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs",
  "rs", "py", "rb", "go", "java", "kt", "kts", "c", "cc", "cpp", "cxx", "h", "hpp", "cs", "php", "swift", "scala", "lua", "pl", "m", "mm",
  "sh", "bash", "zsh", "fish", "ps1", "psm1", "bat", "cmd",
  "css", "scss", "sass", "less", "html", "htm", "xhtml", "xml", "svg", "vue", "svelte", "astro",
  "json", "json5", "jsonc", "yaml", "yml", "toml", "ini", "cfg", "conf", "env", "properties", "lock",
  "md", "mdx", "markdown", "rst", "txt", "text", "adoc", "tex",
  "sql", "sqlite", "sqlite3", "db", "csv", "tsv",
  "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "rtf", "epub",
  "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "ico", "avif", "heic",
  "mp3", "mp4", "wav", "flac", "ogg", "mov", "avi", "mkv", "webm",
  "zip", "tar", "gz", "tgz", "bz2", "xz", "rar", "7z", "jar", "war",
  "exe", "dll", "so", "dylib", "o", "a", "class", "bin", "wasm",
  "log", "map", "tmp", "bak", "swp", "orig", "rej", "patch", "diff",
  "woff", "woff2", "ttf", "otf", "eot",
]);

/** One URL found in a longer string, as offsets into that string. */
export type UrlMatch = { start: number; end: number; url: string };

/** The URL a single token is, or `null` when it is prose or a filename. */
export function asNavigableUrl(token: string): string | null {
  if (!token) return null;
  if (/^https?:\/\//i.test(token)) return httpUrlOrNull(token);
  // Plain `http`, because a dev server prints `http://localhost:5173` -- assuming
  // `https` here would make the one address this shape exists for refuse to load.
  if (LOCAL_HOST.test(token)) return httpUrlOrNull(`http://${token}`);
  if (!BARE_HOST.test(token)) return null;
  const url = httpUrlOrNull(`https://${token}`);
  const label = url && lastLabel(new URL(url).hostname);
  return label && !FILE_EXTENSIONS.has(label) ? url : null;
}

/** The address a whole selection is, if it is one. */
export function urlFromSelection(text: string): string | null {
  const candidate = text.trim();
  // A URL never contains whitespace, so a multi-word highlight is never one --
  // which is also what stops a selected sentence that happens to end in a link.
  if (!candidate || /\s/.test(candidate)) return null;
  return asNavigableUrl(candidate);
}

/**
 * Every URL in `text`, in order and never overlapping.
 *
 * `start`/`end` are offsets so a caller can keep the original wording around each
 * link; `url` is the address to open. This is what makes a fenced code block --
 * which markdown leaves literal by definition -- show the addresses in it.
 */
export function findUrls(text: string): UrlMatch[] {
  const found: UrlMatch[] = [];
  // `matchAll` walks a clone, so the shared pattern carries no cursor between
  // calls and a scan can never start part-way through the next one.
  for (const match of text.matchAll(CANDIDATE)) {
    const core = match[0].replace(TRAILING_PUNCTUATION, "");
    if (!core) continue;
    const url = asNavigableUrl(core);
    if (url) found.push({ start: match.index, end: match.index + core.length, url });
  }
  return found;
}

/**
 * The domain's last label, or `null` when the host is not a plausible one.
 *
 * An empty label -- `etc.` parses to the hostname `etc.` -- and a purely numeric
 * one are both rejected, which is what keeps a trailing-period abbreviation and a
 * version like `1.2` from reading as a domain.
 */
function lastLabel(hostname: string): string | null {
  const labels = hostname.split(".");
  const last = labels[labels.length - 1] ?? "";
  return /^[a-z]{2,24}$/i.test(last) ? last.toLowerCase() : null;
}

/**
 * Parses `value`, keeping it only if the panel's webview could actually open it.
 *
 * The same `http`/`https`-only rule the backend enforces (see `panel/browser`),
 * applied here so a link is never offered for an address that would be refused on
 * arrival.
 */
function httpUrlOrNull(value: string): string | null {
  try {
    const url = new URL(value);
    return url.protocol === "http:" || url.protocol === "https:" ? url.href : null;
  } catch {
    return null;
  }
}
