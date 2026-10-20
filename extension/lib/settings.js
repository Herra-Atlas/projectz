/**
 * What this extension remembers, in one place.
 *
 * Three surfaces write the same record -- the service worker, the popup, the options
 * page -- and a key spelled differently in one of them is a setting that silently does
 * nothing. `chrome.storage.local` rather than `sync` on purpose: the token is a secret
 * for this machine, and syncing it to a Google account is the opposite of what it is for.
 *
 * The record is deliberately small, and split by *who owns what*:
 *
 * - `token` and `port` are *pairing*: the address of the app, handed over by the page the
 *   app serves.
 * - `enabled`, `display`, `limits`, `usage` and `protocol` are *mirrors* of facts the app
 *   owns, written here by the app's frames and never by this extension's own settings --
 *   the master switch belongs to the app, and a second copy that could disagree with it
 *   would be a bug with no good answer. The mirrors exist so a page, a popup or a note can
 *   read them without a round trip.
 * - `auto`, `settle`, `files`, `maxBlocks`, `maxBlockChars`, `cardStyle`, `card`,
 *   `opacity`, `hideAfter` and `sites` are *this extension's*: when to look, how much of a
 *   page may cross the wire, and how an answer is drawn. They are about the browser, and
 *   the app has no business having an opinion about them.
 */

/** The version of the frame protocol this build speaks. Must match `assist::source::browser::PROTOCOL`. */
export const PROTOCOL = 5;

export const DEFAULTS = {
  port: 43128,
  token: "",
  /** The app's switch, mirrored. Off until the app says otherwise. */
  enabled: false,
  /** Whether the app wants cards drawn at all (false in its dry-run mode). */
  display: true,
  /** Whether this connection is live. Set by the worker, read by the popup. */
  connected: false,
  /**
   * The floor and the cut the app applies, mirrored from its frames.
   *
   * Here rather than as constants of this extension because there must be one floor and not
   * two: the app ignores a block under its `minChars`, so sending one is a wasted trip, and a
   * user who raises that floor in Settings must see the browser stop sending short blocks
   * rather than only the model stop answering them.
   */
  limits: { minChars: 6, maxAskChars: 1200 },
  /** What this hour has spent, and the ceiling it is counted against, both the app's. */
  usage: { used: 0, limit: 30 },
  /** The protocol the app said it speaks, for telling the user the two are out of step. */
  protocol: 0,
  /** Whether to read pages without being asked. Off means the popup's button only. */
  auto: true,
  /** How long the page must be still before it is read, in milliseconds. */
  settle: 900,
  /**
   * Whether `file://` pages may be read.
   *
   * Off by default, and it is not enough on its own: Chrome refuses to inject a content
   * script into a local file until the user has turned on "Allow access to file URLs" for
   * this extension. A local file is also the most private thing a browser opens, so both
   * switches are deliberate.
   */
  files: false,
  /** Most blocks one look may send. */
  maxBlocks: 60,
  /** Most text one block may contribute, in characters. */
  maxBlockChars: 600,
  /** `card` draws an answer beside the text; `inline` appends it to the text itself. */
  cardStyle: "card",
  /** Where a card goes when the block has no room: `auto`, `above`, `below`. */
  card: "auto",
  /** How solid a card is, 0.6 to 1. Below about 0.85 the page shows through the words. */
  opacity: 0.92,
  /** Seconds before a card fades on its own. Zero keeps it until it is dismissed. */
  hideAfter: 0,
  /** Sites to leave alone: hostname -> `off`. Anything absent is read as usual. */
  sites: {},
};

/** Reads the whole record, filling in anything missing. */
export async function load() {
  const stored = await chrome.storage.local.get(DEFAULTS);
  // A stored record may predate a key, and `get` with defaults covers that -- but a
  // stored `sites` that is not an object (a hand-edited record) would break the lookups
  // below, so it is checked rather than trusted.
  return {
    ...DEFAULTS,
    ...stored,
    sites: stored && typeof stored.sites === "object" && stored.sites ? stored.sites : {},
  };
}

/** Writes a patch over the record. */
export async function save(patch) {
  await chrome.storage.local.set(patch);
  // Storage change events are how every other surface learns about a write, so nothing
  // here needs to tell them itself.
  return patch;
}

/** The hostname of a URL, or empty when it cannot be read. */
export function hostOf(url) {
  try {
    return new URL(url).hostname;
  } catch {
    return "";
  }
}

/** The scheme of a URL, or empty when it cannot be read. */
export function schemeOf(url) {
  try {
    return new URL(url).protocol;
  } catch {
    return "";
  }
}

/**
 * Whether a page is one this extension may read at all.
 *
 * A scheme allowlist, because "everything else" is a longer list than it looks: `chrome://`,
 * `chrome-extension://`, `devtools://`, `view-source:` and a browser's own pages are not
 * content, and some of them are the browser's own settings. `file:` is allowed only when the
 * user has asked for it here *and* Chrome has been given the grant.
 */
export function readableUrl(url, state) {
  const scheme = schemeOf(url);
  if (scheme === "http:" || scheme === "https:") return true;
  return scheme === "file:" && Boolean(state && state.files);
}

/** The address of the app's socket, for the given record. */
export function socketUrl(state) {
  return `ws://127.0.0.1:${state.port}`;
}

/** The app's own setup page, for the same host and port. */
export function setupUrl(state) {
  return `http://127.0.0.1:${state.port}/`;
}
