/**
 * The capabilities a run may hold.
 *
 * Mirrors the Rust `AccessSet` field for field. Kept in one shared module because
 * jobs, Telegram and the installation-wide host switches are the same switches
 * with different owners; three copies would drift the first time a capability was
 * added.
 */
export type AccessSet = {
  read: boolean;
  write: boolean;
  terminal: boolean;
  web: boolean;
  subagents: boolean;
  /** Scheduling work for later. A job's own run never has it. */
  jobs: boolean;
  /** Tools offered by the configured MCP servers. */
  mcp: boolean;
};

export type AccessCapability = keyof AccessSet;

/** Everything on, which is what a hand-started run gets. */
export const ALL_ACCESS: AccessSet = {
  read: true,
  write: true,
  terminal: true,
  web: true,
  subagents: true,
  jobs: true,
  mcp: true,
};

/** The order every access editor lists them in. */
export const ACCESS_CAPABILITIES: AccessCapability[] = [
  "read",
  "write",
  "terminal",
  "web",
  "subagents",
  "jobs",
  "mcp",
];

/** What each switch is called when a row needs more than the field name. */
export const ACCESS_LABELS: Record<AccessCapability, string> = {
  read: "Read",
  write: "Write",
  terminal: "Terminal",
  web: "Web",
  subagents: "Subagents",
  jobs: "Jobs",
  mcp: "MCP",
};
