/** Which tools a reply may use.
 *
 * Sent on the request rather than resolved in the backend, so the mode a run
 * claims is the mode it gets. `Chat` sends no `tools` array at all.
 *
 * The wire names are lowercase and must match the Rust enum's serde
 * representation. Adding a mode means adding it there too; a mismatch here
 * deserializes to `Chat`, which fails safe. */
export type ChatMode = "chat" | "agent";

/** How much the agent may do without asking.
 *
 * Wire names match `PermissionMode` in Rust. `ask` is the default and the safe
 * direction to fall back to: a setting that is missing or unreadable must never
 * become "run everything". */
export type PermissionMode = "ask" | "auto_safe" | "auto_writes" | "full";

/** A conversation's working mode and access level, as one value.
 *
 * The two are chosen together and always travel together: a level is only
 * offered in Agent mode, so the pair is what a conversation actually is being
 * worked as. Passing them separately would let a caller record an access level
 * against a conversation that is not in Agent mode.
 */
export type WorkingMode = { mode: ChatMode; permission: PermissionMode };

/** The permission level in force for the next reply. */
export const DEFAULT_PERMISSION: PermissionMode = "ask";

/** Human labels for the permission selector, in the order they are offered.
 *
 * Ascending by how much they allow, so the list reads as a single scale rather
 * than as three settings and a wildcard. `Auto-approve reads` is listed third
 * rather than first on purpose: it is the setting a power user wants, not the
 * one a new user should land on. */
export const PERMISSION_LABELS: Record<PermissionMode, string> = {
  ask: "Ask each time",
  auto_safe: "Auto-approve reads",
  auto_writes: "Auto-approve writes",
  full: "Full access",
};

/** One line explaining which tools a level approves automatically. */
export const PERMISSION_HINTS: Record<PermissionMode, string> = {
  ask: "Ask you before every tool call.",
  auto_safe: "Auto-approve list_dir, read_file, search_files, and skill_read. Ask before every other tool.",
  auto_writes: "Also auto-approve write_file, edit_file, edit_lines, and skill_manage. Ask before every other tool.",
  full: "Auto-approve every tool call.",
};

/** The collapsed selector's value, where the full label will not fit.
 *
 * Its own table rather than a ternary at the call site: four levels make a
 * nested conditional long enough to be worth a lookup, and a level added later
 * becomes one entry here instead of a fourth branch to remember. */
export const ACCESS_SHORT: Record<PermissionMode, string> = {
  ask: "Ask",
  auto_safe: "Auto reads",
  auto_writes: "Auto writes",
  full: "Full",
};

/** Settings key holding the permission level. */
export const PERMISSION_SETTING_KEY = "app.agent_permission";

/** True when the backend accepts this spelling of a permission level.
 *
 * Guards against a value reaching `invoke` that Rust would reject: an unknown
 * string deserializes to the default anyway, so sending one buys nothing and a
 * mismatch here would be silent. */
export function isPermissionMode(value: unknown): value is PermissionMode {
  return value === "ask" || value === "auto_safe" || value === "auto_writes" || value === "full";
}

/** Reads a stored permission level, falling back to the safest one. */
export function permissionOrDefault(value: unknown): PermissionMode {
  return isPermissionMode(value) ? value : DEFAULT_PERMISSION;
}