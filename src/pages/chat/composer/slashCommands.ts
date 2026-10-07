/**
 * The slash commands the composer understands.
 *
 * A command is a whole draft: the user types `/name` and it either runs outright
 * (Enter) or is picked from the menu. It is deliberately not a message prefix --
 * a command changes how the *next send* behaves rather than being sent, so the
 * draft is cleared once one runs.
 *
 * Kept as data with a named `action` rather than a rendered element or a closure:
 * the registry stays pure and testable, and the one component that owns the state
 * a command touches (the composer) switches on the action.
 */

/** What a command does. One variant per thing a command can change. */
export type SlashAction = "web-search-on" | "web-search-off";

export type SlashCommand = {
  /** Typed after the slash, e.g. `/web-on`. */
  name: string;
  /** Menu title. */
  label: string;
  /** One line under the title, shown in the menu. */
  hint: string;
  action: SlashAction;
};

/** The commands offered, in menu order. */
export const SLASH_COMMANDS: SlashCommand[] = [
  {
    name: "web-on",
    label: "Web search on",
    hint: "Let this reply search the web",
    action: "web-search-on",
  },
  {
    name: "web-off",
    label: "Web search off",
    hint: "Answer without searching the web",
    action: "web-search-off",
  },
];

/**
 * The query while a command is being typed: `/web` gives `web`.
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

/** Commands whose name still matches the typed query, in menu order. */
export function matchCommands(query: string): SlashCommand[] {
  const needle = query.trim().toLowerCase();
  return SLASH_COMMANDS.filter((command) => command.name.startsWith(needle));
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
  return SLASH_COMMANDS.find((command) => command.name === name) ?? null;
}
