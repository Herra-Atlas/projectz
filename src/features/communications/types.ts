/**
 * The messaging feature's shapes, mirroring the Rust records.
 *
 * One record and a list of channels, rather than one type per transport: adding
 * Discord later adds a row to `channels`, not a new type here. A channel's own
 * configuration is opaque to this file on purpose -- the page draws the Telegram
 * section and reads `config.tokenSaved`, and nothing above that needs to know what
 * a bot token is.
 */

import type { AccessSet } from "../chat/access";

export type MessagingMode = "chat" | "agent";

/** The levels a run can decide at. `auto_safe` is reachable through the record but
 *  not offered by the page: a message-run almost always wants to write something,
 *  and the two-level choice is the one a person can reason about. */
export type MessagingPermission = "ask" | "auto_safe" | "auto_writes" | "full";

/** What one Telegram id may do. */
export type MessagingUser = {
  mode: MessagingMode;
  allowAgent: boolean;
  permission: MessagingPermission;
  access: AccessSet;
  allowLocalStart: boolean;
  allowSttStart: boolean;
};

export type MessagingSettings = {
  enabled: boolean;
  /** The Telegram user ids the bot obeys. Empty means nobody. */
  allowedUserIds: string[];
  /** One record per allowed id. */
  users: Record<string, MessagingUser>;
  channels: string[];
  /** Whether files may be received from and sent to the chat. */
  allowFiles: boolean;
  /** Extra instructions for every message answered through a channel. */
  instructions: string;
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
