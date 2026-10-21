/**
 * The startup argument list, as text a person can edit.
 *
 * The list is the one thing on the model page that is not a dial, so it has to
 * survive being typed in by hand and then being poked by a dial afterwards. These
 * three functions are that contract: split what the user typed, quote it back
 * without changing it, and touch exactly one flag when a setting moves.
 */

/** One flag the page knows how to place in a custom list. */
export type ArgumentSpec = {
  flag: string;
  /** A flag with no value, present or absent. */
  isSwitch?: boolean;
};

/**
 * Splits a command line into arguments.
 *
 * Quoted spans stay one argument, because a template is allowed to contain
 * spaces and splitting on them would silently turn one argument into five.
 */
export function splitArguments(text: string): string[] {
  const tokens: string[] = [];
  let current = "";
  let quote: '"' | "'" | null = null;
  let started = false;

  for (const character of text) {
    if (quote) {
      if (character === quote) {
        quote = null;
      } else {
        current += character;
      }
      continue;
    }
    if (character === '"' || character === "'") {
      quote = character;
      started = true;
      continue;
    }
    if (/\s/.test(character)) {
      if (started) {
        tokens.push(current);
        current = "";
        started = false;
      }
      continue;
    }
    current += character;
    started = true;
  }
  if (started) tokens.push(current);
  return tokens;
}

/** Puts an argument list back on one line, quoting only what needs it. */
export function quoteArguments(tokens: string[]): string {
  return tokens
    .map((token) =>
      token === "" || /[\s"']/.test(token)
        ? `"${token.replace(/"/g, '\\"')}"`
        : token,
    )
    .join(" ");
}

/**
 * Sets one flag in a hand-written list and leaves every other argument alone.
 *
 * A dial whose flag is absent is appended rather than ignored: a user who writes
 * their own list still expects the context slider to do something. The reverse
 * holds too -- turning a switch off removes its flag instead of writing
 * `--mlock false`, which llama.cpp would read as a filename.
 */
export function syncArgument(
  text: string,
  spec: ArgumentSpec,
  value: string | null,
): string {
  const tokens = splitArguments(text);
  const at = tokens.indexOf(spec.flag);

  if (spec.isSwitch) {
    if (value !== null && at < 0) tokens.push(spec.flag);
    if (value === null && at >= 0) tokens.splice(at, 1);
    return quoteArguments(tokens);
  }

  if (value === null) {
    if (at < 0) return quoteArguments(tokens);
    tokens.splice(at, at + 1 < tokens.length ? 2 : 1);
    return quoteArguments(tokens);
  }

  if (at < 0) {
    tokens.push(spec.flag, value);
  } else if (at + 1 < tokens.length) {
    tokens[at + 1] = value;
  } else {
    tokens.push(value);
  }
  return quoteArguments(tokens);
}
