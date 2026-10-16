import type { McpServer, McpServerView } from "./types";

/**
 * A server while it is being written.
 *
 * `command` is the whole command line and `env` is free text, rather than the
 * arrays they are stored as: a form is a place for writing, not for managing
 * lists, and a command's arguments are typed the way they are typed in a
 * terminal. The conversion happens once, on save -- which is also where a blank
 * line can be dropped instead of becoming an empty argument.
 */
export type McpDraft = {
  id: string;
  name: string;
  /** The program and its arguments, or a URL. */
  command: string;
  /** The credential, or empty. */
  key: string;
  /** One `NAME=value` per line. */
  env: string;
  enabled: boolean;
};

export const NEW_DRAFT: McpDraft = {
  id: "",
  name: "",
  command: "",
  key: "",
  env: "",
  enabled: true,
};

/** The editable form of a stored server. */
export function draftFrom(server: McpServerView): McpDraft {
  return {
    id: server.id,
    name: server.name,
    command: commandLine(server),
    key: server.key,
    env: server.env.map(([name, value]) => `${name}=${value}`).join("\n"),
    enabled: server.enabled,
  };
}

/** Every line, trimmed, with the blanks dropped. */
const lines = (text: string) =>
  text
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0);

/** The server a draft describes, ready to save. */
export function serverFrom(draft: McpDraft): McpServer {
  const [command = "", ...args] = splitCommandLine(draft.command);
  return {
    id: draft.id,
    name: draft.name.trim(),
    command,
    args,
    key: draft.key.trim(),
    env: lines(draft.env).map((line): [string, string] => {
      const at = line.indexOf("=");
      // A line with no `=` is a name set to nothing, which is a real thing to do
      // and better than silently dropping what was typed.
      return at === -1
        ? [line, ""]
        : [line.slice(0, at).trim(), line.slice(at + 1).trim()];
    }),
    enabled: draft.enabled,
  };
}

/** A part of a command line, quoted if it has to be to survive the split. */
const quoted = (part: string) => (/\s/.test(part) ? `"${part}"` : part);

/** The command as it would be typed, for the form and the row's tooltip. */
export function commandLine(server: McpServer): string {
  return [server.command, ...server.args].filter(Boolean).map(quoted).join(" ");
}

/**
 * A command line split the way the shell that will run it would split it.
 *
 * Double quotes group and are removed; everything else to a whitespace boundary
 * is one part. **Not** a general shell parser, deliberately: this runs on a
 * Windows-first app, where a backslash is a path separator rather than an escape,
 * so treating `\` as escaping would shorten `"C:\Program Files\node.exe"` into
 * something that does not exist. A quote character cannot appear in a Windows
 * path, which is what makes the simple rule sufficient here.
 */
export function splitCommandLine(text: string): string[] {
  const parts: string[] = [];
  let current = "";
  let grouping = false;

  for (const character of text.trim()) {
    if (character === '"') {
      grouping = !grouping;
      continue;
    }
    if (!grouping && /\s/.test(character)) {
      if (current.length > 0) parts.push(current);
      current = "";
      continue;
    }
    current += character;
  }
  if (current.length > 0) parts.push(current);
  return parts;
}
