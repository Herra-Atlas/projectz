/**
 * The messaging feature's shapes, mirroring the Rust records.
 *
 * One record and a list of channels, rather than one type per transport: adding
 * Discord later adds a row to `channels`, not a new type here. A channel's own
 * configuration is opaque to this file on purpose -- the page draws the Telegram
 * section and reads `config.tokenSaved`, and nothing above that needs to know what
 * a bot token is.
 */

export type MessagingMode = "chat" | "agent";

/** The levels a run can decide at. `auto_safe` is reachable through the record but
 *  not offered by the page: a message-run almost always wants to write something,
 *  and the two-level choice is the one a person can reason about. */
export type MessagingPermission = "ask" | "auto_safe" | "auto_writes" | "full";

export type MessagingAccess = {
  read: boolean;
  write: boolean;
  terminal: boolean;
  web: boolean;
  subagents: boolean;
  /** Always false: a message may not schedule work that outlives it. */
  jobs: boolean;
};

export type MessagingSettings = {
  enabled: boolean;
  /** The Telegram user ids the bot obeys. Empty means nobody. */
  allowedUserIds: string[];
  channels: string[];
  mode: MessagingMode;
  permission: MessagingPermission;
  access: MessagingAccess;
  /** Whether a chat may start a local model that is not loaded. */
  allowLocalStart: boolean;
  workspace: string | null;
};

export type ChannelView = {
  id: string;
  name: string;
  enabled: boolean;
  running: boolean;
  /** The channel's own name for itself, such as `@projectz_bot`. */
  identity: string | null;
  error: string | null;
  /** Redacted by the backend; a secret is never in here. */
  config: { tokenSaved?: boolean } & Record<string, unknown>;
};

export type MessagingStatus = {
  settings: MessagingSettings;
  channels: ChannelView[];
};

/** The event the backend raises when the record or a conversation changes. */
export type MessagingEvent = { kind: "state"; sessionId?: string };
