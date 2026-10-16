/** One configured MCP server, as it is stored. */
export type McpServer = {
  /** Stable id, empty for a server that has not been saved yet. */
  id: string;
  name: string;
  /** The program to run, resolved on `PATH`. */
  command: string;
  args: string[];
  /** `NAME`/value pairs handed to the child process. */
  env: [string, string][];
  /**
   * The credential, or empty.
   *
   * A bare value is handed to a command as `MCP_API_KEY`, or sent as a bearer
   * token to a URL; `NAME=value` picks the environment variable instead.
   */
  key: string;
  enabled: boolean;
};

/** One tool a server advertised. */
export type McpToolSummary = {
  /** The name the model calls, e.g. `mcp__files__read_file`. */
  name: string;
  /** The server's own name for it, which is what a call sends. */
  remote_name: string;
  description: string;
};

/**
 * A server plus what happened the last time it was reached.
 *
 * The server's own fields are at the top level rather than nested: the backend
 * flattens the view on the way out, so one shape travels both ways.
 */
export type McpServerView = McpServer & {
  tools: McpToolSummary[];
  /** The last failure, or empty when the server answered. */
  error: string;
};
