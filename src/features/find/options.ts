/**
 * What counts as a match.
 *
 * Three switches rather than a syntax: the find bar is used mid-reply by someone
 * who is looking for a word they can see on screen, and a regular expression
 * field would make the common case (type a word, press Enter) carry an escaping
 * rule. Each one is persisted, because a search that has to be re-configured
 * every time is one that gets configured wrong.
 */
export type FindOptions = {
  /** `Foo` only finds `Foo`. Off, it finds `foo` and `FOO` too. */
  caseSensitive: boolean;
  /** `cat` does not find the `cat` inside `concatenate`. */
  wholeWord: boolean;
  /** `a` finds `á`, `à` and `ä`; on by default, since accents are rarely the
      thing being searched for and almost never worth typing. */
  ignoreAccents: boolean;
};

export const DEFAULT_FIND_OPTIONS: FindOptions = {
  caseSensitive: false,
  wholeWord: false,
  ignoreAccents: true,
};

/**
 * Where the switches are remembered.
 *
 * `localStorage` rather than the settings database, matching the other
 * frontend-only choices (`projectz.selected-model.v1`): nothing in Rust reads
 * this, and a search option is a habit rather than a record worth migrating.
 */
const STORAGE_KEY = "projectz.find.v1";

/** A stored field is only used when it is the type it claims to be. */
function readFlag(value: unknown, fallback: boolean): boolean {
  return typeof value === "boolean" ? value : fallback;
}

export function loadFindOptions(): FindOptions {
  try {
    const stored = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "null") as Record<string, unknown> | null;
    if (!stored) return DEFAULT_FIND_OPTIONS;
    return {
      caseSensitive: readFlag(stored.caseSensitive, DEFAULT_FIND_OPTIONS.caseSensitive),
      wholeWord: readFlag(stored.wholeWord, DEFAULT_FIND_OPTIONS.wholeWord),
      ignoreAccents: readFlag(stored.ignoreAccents, DEFAULT_FIND_OPTIONS.ignoreAccents),
    };
  } catch {
    // Unreadable or not JSON. The defaults are a working find bar, which is what
    // an unreadable preference must never cost.
    return DEFAULT_FIND_OPTIONS;
  }
}

export function saveFindOptions(options: FindOptions): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(options));
  } catch {
    // A full or unavailable store is not worth reporting: the choice still
    // applies to this session, and nothing else depends on the mirror.
  }
}
