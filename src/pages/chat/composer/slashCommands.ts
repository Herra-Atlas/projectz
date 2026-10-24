/**
 * The slash commands the composer understands.
 *
 * A command is a whole draft: the user types `/name` and it either runs outright
 * (Enter) or is picked from the menu. It is deliberately not a message prefix --
 * a command changes how the *next send* behaves rather than being sent, so the
 * draft is cleared once one runs.
 *
 * Kept as data rather than a rendered element or a closure: the registry stays
 * pure and testable, and the one component that owns the state a command touches
 * (the composer) switches on the command.
 */

export type SlashCommand = {
  /** Typed after the slash, e.g. `/fork`. */
  name: string;
  /** Other spellings that run the same command. */
  aliases?: string[];
  /** Menu title. */
  label: string;
  /** One line under the title, shown in the menu. */
  hint: string;
  /**
   * A placeholder is drawn but cannot be run. The menu carries one while there
   * is nothing to offer, so `/` says something instead of opening an empty box.
   */
  disabled?: boolean;
};

/**
 * The commands offered, in menu order.
 *
 * Each one has to be something that has no control beside the field. `/compact` is the
 * first because it is the clearest case: automatic compaction only fires from inside a
 * reply, at 80% of the window, and a user who can feel a conversation getting heavy had no
 * way to act on that between replies. A command that only repeated a button would be a
 * second door into the same room; this one is the only door.
 */
export const SLASH_COMMANDS: SlashCommand[] = [
  {
    name: "compact",
    // The word people reach for. `compact` is the stored name and the one the
    // backend command uses, so both are accepted and the menu shows one row.
    aliases: ["compress"],
    label: "Compact this conversation",
    hint: "Trim old tool output so the next reply fits its window",
  },
];

/**
 * The query while a command is being typed: `/ag` gives `ag`.
 *
 * `null` for anything that is not a bare leading slash-word, which is what closes
 * the menu -- a space or a newline means the rest is a message, not a name.
 */
export function slashQuery(draft: string): string | null {
  if (!draft.startsWith("/")) return null;
  const rest = draft.slice(1);
  if (rest.includes(" ") || rest.includes("\n") || rest.includes("\t")) return null;
  return rest;
}

/**
 * Commands matching the typed query, in menu order.
 *
 * A bare `/` matches everything, the placeholder included -- that is the one
 * moment it is worth showing. The first letter typed drops it out, because a name
 * it cannot be run by is not a match.
 */
export function matchCommands(query: string): SlashCommand[] {
  const needle = query.trim().toLowerCase();
  if (needle.length === 0) return SLASH_COMMANDS;
  return SLASH_COMMANDS.filter((command) => !command.disabled && (command.name.startsWith(needle) || (command.aliases ?? []).some((alias) => alias.startsWith(needle))));
}

/**
 * The command a draft is, when the whole draft is exactly one.
 *
 * This is the state the composer draws as a link: a finished command is not a
 * message, and styling it apart from prose is what says so. A trailing space is
 * tolerated, because a user who has finished typing the word expects it to have
 * taken effect.
 */
export function exactCommand(draft: string): SlashCommand | null {
  const query = slashQuery(draft.trimEnd());
  if (query === null || query.length === 0) return null;
  const name = query.toLowerCase();
  return SLASH_COMMANDS.find((command) => !command.disabled && (command.name === name || (command.aliases ?? []).includes(name))) ?? null;
}
