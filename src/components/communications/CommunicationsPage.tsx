import { useState } from "react";
import { LoaderCircle, Trash2 } from "lucide-react";

import { HoldToDelete } from "../HoldToDelete";
import {
  ANCHOR_ATTRIBUTE,
  FlashHighlight,
  MARKABLE_ATTRIBUTE,
  Segmented,
  SettingRow,
  SettingsPageTitle,
  SettingsSection,
  Toggle,
} from "../SettingsSection";
import { AllowlistEditor } from "./AllowlistEditor";
import { ACCESS_SHORT, PERMISSION_HINTS } from "../../features/chat/chatMode";
import type {
  MessagingPermission,
  MessagingSettings,
} from "../../features/communications/types";
import type { MessagingStore } from "../../features/communications/useMessaging";
import type { Notify } from "../../features/notifications/types";

const MODES = ["Agent", "Chat"] as const;
/** The same four levels the composer offers, in the same order and wording. */
const PERMISSIONS = ["Ask", "Auto reads", "Auto writes", "Full"] as const;

/** A compact permission label back to the value the backend stores. */
function permissionValue(label: (typeof PERMISSIONS)[number]): MessagingPermission {
  if (label === "Ask") return "ask";
  if (label === "Auto reads") return "auto_safe";
  if (label === "Auto writes") return "auto_writes";
  return "full";
}

type CommunicationsPageProps = {
  notify: Notify;
  /** Read by the settings modal when the dialog opens, so the page lands on data. */
  store: MessagingStore;
};

/**
 * Messaging, as one page.
 *
 * The order is the order it is set up in: switch it on, connect a transport, say who
 * may use it, then choose what answers. Every channel is a section, so adding Discord
 * later is another section here rather than a second page in the sidebar.
 *
 * **The allowlist is the security boundary, and the page says so.** A bot that can
 * run tools is a remote shell with a nicer voice; an empty list means nobody, and a
 * reader who thinks it means everybody would leave the door open.
 */
export function CommunicationsPage({ notify, store }: CommunicationsPageProps) {
  const settings = store.status?.settings ?? null;
  const channel = store.status?.channels.find((entry) => entry.id === "telegram") ?? null;
  const [token, setToken] = useState("");
  const [savingToken, setSavingToken] = useState(false);

  if (!settings) {
    return (
      <>
        <SettingsPageTitle>
          <h2 className="text-[17px] font-semibold tracking-tight">Communications</h2>
        </SettingsPageTitle>
        <p className="px-4 py-6 text-sm text-[var(--muted)]">
          {store.error || "Reading settings…"}
        </p>
      </>
    );
  }

  /** Writes a change and reports a save that did not happen. */
  const commit = (patch: Partial<MessagingSettings>) => {
    void store.save({ ...settings, ...patch }).then((next) => {
      if (!next) notify("error", "That could not be saved.");
    });
  };

  const saveToken = async () => {
    setSavingToken(true);
    try {
      await store.saveChannel("telegram", { token });
      setToken("");
      notify("success", "Telegram bot connected.");
    } catch (reason) {
      notify("error", String(reason));
    } finally {
      setSavingToken(false);
    }
  };

  const forgetToken = async () => {
    try {
      await store.saveChannel("telegram", { clear: true });
      notify("success", "The bot token was removed.");
    } catch (reason) {
      notify("error", String(reason));
    }
  };

  const tokenSaved = channel?.config.tokenSaved === true;
  const identity = channel?.identity ?? null;
  const status = channel?.error
    ? channel.error
    : channel?.running
      ? "Listening for messages."
      : tokenSaved
        ? "Saved. It connects when messaging is on."
        : "No bot token yet.";

  return (
    <>
      <SettingsPageTitle>
        <h2 className="text-[17px] font-semibold tracking-tight">Communications</h2>
        {/* The master switch lives in the band, like Assist's: it gates every
            channel below, so it belongs above them rather than inside one. */}
        <div
          className="flex items-center gap-2.5"
          {...{ [ANCHOR_ATTRIBUTE]: "communications.enabled", [MARKABLE_ATTRIBUTE]: "" }}
        >
          <FlashHighlight anchor="communications.enabled" />
          <span className="text-xs text-[var(--muted)]">{settings.enabled ? "On" : "Off"}</span>
          <Toggle
            checked={settings.enabled}
            onChange={(next) => void store.setEnabled(next)}
            label="Messaging"
          />
        </div>
      </SettingsPageTitle>

      <SettingsSection title="Telegram">
        <SettingRow
          anchor="communications.telegram.status"
          label={identity ? `Connected as ${identity}` : "Telegram"}
          description={status}
          control={
            <span
              aria-hidden="true"
              className={`size-2 rounded-full ${
                channel?.error
                  ? "bg-[var(--danger)]"
                  : channel?.running
                    ? "bg-[var(--accent)]"
                    : "bg-[var(--quiet)]"
              }`}
            />
          }
          hint="Create a bot with @BotFather in Telegram, paste the token here, then message the bot to link your id."
        />

        <SettingRow
          anchor="communications.telegram.token"
          label="Bot token"
          description={
            tokenSaved
              ? "A token is saved. Pasting a new one replaces it."
              : "From @BotFather. It is stored in this app's database and never shown again."
          }
          stacked
          control={
            <div className="flex items-center gap-2">
              <input
                type="password"
                value={token}
                onChange={(event) => setToken(event.target.value)}
                autoComplete="off"
                spellCheck={false}
                aria-label="Bot token"
                placeholder={tokenSaved ? "•••••••• saved" : "123456:ABC-DEF…"}
                className="min-h-9 flex-1 rounded-lg border border-[var(--line)] bg-[var(--panel)] px-2.5 text-[13px] text-[var(--text)] outline-none transition-colors focus:border-[var(--accent)]"
              />
              <button
                type="button"
                onClick={() => void saveToken()}
                disabled={!token.trim() || savingToken}
                className="inline-flex min-h-9 items-center gap-1.5 rounded-lg border border-[var(--line)] bg-[var(--panel)] px-3 text-[13px] font-medium text-[var(--text)] transition-colors hover:border-[var(--line-strong)] hover:bg-[var(--raised)] disabled:opacity-40"
              >
                {savingToken && <LoaderCircle size={14} className="animate-spin" />}
                Save
              </button>
              {tokenSaved && (
                <HoldToDelete
                  label="Forget the bot token"
                  title="Hold to remove the saved token"
                  onConfirm={() => void forgetToken()}
                  className="grid size-9 place-items-center rounded-lg border border-[var(--line)] text-[var(--muted)] transition-colors hover:border-[var(--danger)] hover:text-[var(--danger)]"
                >
                  <Trash2 size={15} />
                </HoldToDelete>
              )}
            </div>
          }
        />

        <SettingRow
          anchor="communications.telegram.allowed"
          label="Who may message"
          description="Telegram user ids. Empty means nobody — the bot explains itself to a stranger only while this list is empty, and replies with the id to add."
          stacked
          control={
            <AllowlistEditor
              ids={settings.allowedUserIds}
              onChange={(allowedUserIds) => commit({ allowedUserIds })}
              notify={notify}
            />
          }
          hint="Message the bot from the phone you want to use and it answers with your numeric id. Usernames are refused: a username can change hands, an id cannot."
        />
      </SettingsSection>

      <SettingsSection title="How it answers">
        <SettingRow
          anchor="communications.mode"
          label="Agent or chat"
          description={
            settings.mode === "agent"
              ? "Tools, files and the workspace. This is the one that can do work."
              : "A conversation and nothing more: no tools, no files."
          }
          control={
            <Segmented
              value={settings.mode === "agent" ? "Agent" : "Chat"}
              options={MODES}
              onChange={(label) => commit({ mode: label === "Agent" ? "agent" : "chat" })}
              label="Mode"
            />
          }
        />

        <SettingRow
          anchor="communications.permission"
          label="What it may do unattended"
          description={PERMISSION_HINTS[settings.permission]}
          control={
            <Segmented
              value={ACCESS_SHORT[settings.permission] as (typeof PERMISSIONS)[number]}
              options={PERMISSIONS}
              onChange={(label) => commit({ permission: permissionValue(label) })}
              label="Permission"
            />
          }
          hint="The same four levels the composer offers. Ask works here because a prompt is relayed to the chat as Allow/Deny buttons — a level with nobody to ask would park forever. Full approves everything without a word."
        />

        <SettingRow
          anchor="communications.localStart"
          label="Let Telegram start a local model"
          description="When the model that answers is a local one that is not loaded, the chat offers Start. Loading one replaces whatever the app is using."
          control={
            <Toggle
              checked={settings.allowLocalStart}
              onChange={(next) => commit({ allowLocalStart: next })}
              label="Let Telegram start a local model"
            />
          }
          hint="Off by default: a message can arrive when nobody is at the machine, and starting a model stops the one the window is using. The chat still asks before doing it."
        />
      </SettingsSection>
    </>
  );
}
