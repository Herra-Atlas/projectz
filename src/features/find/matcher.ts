import type { FindOptions } from "./options";

/** One match, as UTF-16 offsets into the text node it was found in. */
export type TextMatch = { start: number; end: number };

/**
 * Text matching, one text node at a time.
 *
 * **Node-local on purpose.** A phrase split across elements (`the <em>quick</em>
 * fox`) is not found as one match. Finding it would mean rebuilding the scope's
 * text with a mapping for every DOM boundary, then splitting the hit back into
 * ranges that may start in one node and end in another -- a large amount of work
 * for a case the user is not usually in. A word, which is what this is for, sits
 * inside one node almost always.
 *
 * **Folding is the diacritics switch.** Accents ignored means every character is
 * decomposed and its combining marks dropped before the comparison, so `é` and
 * `e` are the same character. That changes the string's length, which is why the
 * folded text carries a map back to the original offsets instead of assuming
 * they line up.
 *
 * **The common case skips the map.** Plain ASCII cannot change length under
 * lowercasing and has no accents to strip, so an ASCII node is compared in place
 * and its indices are the original's. A conversation is mostly ASCII prose and
 * code, so this is what keeps a search over a long transcript off the critical
 * path.
 */

const NON_ASCII = /[^\x00-\x7f]/;

/** One combining mark, after NFD. No `g` flag: this one is used with `test`. */
const COMBINING = /\p{M}/u;

/** The same test, global, for stripping a whole string. */
const COMBINING_ALL = /\p{M}/gu;

/** What a word is made of, for the whole-word switch. */
const WORD_CHAR = /[\p{L}\p{N}_]/u;

type FoldedText = {
  text: string;
  /** `null` when folding changed no lengths and folded indices are the original's. */
  offsets: { starts: number[]; ends: number[] } | null;
};

/**
 * Folds `text` for comparison: accents dropped, case lowered, as asked.
 *
 * Entries are pushed one **UTF-16 unit** at a time, not one code point at a time,
 * because `indexOf` reports unit offsets. Pushing a surrogate pair as one entry
 * would shift every index after the first emoji by one.
 */
function fold(text: string, options: FindOptions): FoldedText {
  if (!NON_ASCII.test(text)) {
    return { text: options.caseSensitive ? text : text.toLowerCase(), offsets: null };
  }

  const units: string[] = [];
  const starts: number[] = [];
  const ends: number[] = [];
  let index = 0;

  for (const char of text) {
    const end = index + char.length;
    let piece = options.ignoreAccents ? char.normalize("NFD").replace(COMBINING_ALL, "") : char;
    if (!options.caseSensitive) piece = piece.toLowerCase();
    for (let unit = 0; unit < piece.length; unit += 1) {
      units.push(piece[unit]);
      starts.push(index);
      ends.push(end);
    }
    index = end;
  }

  return { text: units.join(""), offsets: { starts, ends } };
}

/** The character immediately before `index`, without splitting a surrogate pair. */
function charBefore(text: string, index: number): string {
  if (index <= 0) return "";
  const low = text.charCodeAt(index - 1);
  if (low >= 0xdc00 && low <= 0xdfff && index >= 2) return text.slice(index - 2, index);
  return text[index - 1] ?? "";
}

/** Whether a match starting at `start` and ending at `end` stands as a word. */
function standsAlone(folded: string, start: number, end: number): boolean {
  const before = charBefore(folded, start);
  const after = end < folded.length ? String.fromCodePoint(folded.codePointAt(end) ?? 0) : "";
  return !WORD_CHAR.test(before) && !WORD_CHAR.test(after);
}

/**
 * Extends an end offset over combining marks that belong to the last character.
 *
 * Decomposition means the folded match ends before a mark that the reader sees as
 * part of the same letter (`e` + U+0301). Highlighting the base character alone
 * would leave its accent outside the paint.
 */
function includeTrailingMarks(text: string, end: number): number {
  let offset = end;
  while (offset < text.length) {
    const code = text.codePointAt(offset);
    if (code === undefined) break;
    const char = String.fromCodePoint(code);
    if (!COMBINING.test(char)) break;
    offset += char.length;
  }
  return offset;
}

/** Every non-overlapping match of `query` in `text`, in order. */
export function matchText(text: string, query: string, options: FindOptions): TextMatch[] {
  if (!query) return [];

  const folded = fold(text, options);
  const needle = fold(query, options).text;
  if (!needle) return [];

  const matches: TextMatch[] = [];
  let from = 0;
  // `<=` rather than `<` so a needle that is the whole node is still tried, and a
  // shorter one stops the loop without an extra `indexOf`.
  while (from <= folded.text.length - needle.length) {
    const at = folded.text.indexOf(needle, from);
    if (at === -1) break;
    const after = at + needle.length;

    if (!options.wholeWord || standsAlone(folded.text, at, after)) {
      const start = folded.offsets ? folded.offsets.starts[at] : at;
      const rawEnd = folded.offsets ? folded.offsets.ends[after - 1] : after;
      matches.push({ start, end: includeTrailingMarks(text, rawEnd) });
    }

    // Non-overlapping, as every find bar behaves: `aa` in `aaaa` is two matches,
    // not three. `after` is always ahead because `needle` is non-empty.
    from = after;
  }

  return matches;
}
