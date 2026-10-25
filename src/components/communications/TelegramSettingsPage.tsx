import { useState } from "react";
import { LoaderCircle, Trash2 } from "lucide-react";

import { HoldToDelete } from "../HoldToDelete";
import { SettingRow, SettingsSection, Toggle } from "../SettingsSection";
import type { MessagingStore } from "../../features/communications/useMessaging";
import type { Notify } from "../../features/notifications/types";
import {
  CommunicationsLoading,
  CommunicationsTitle,
  saveSettings,
} from "./shared";

/**
 * Telegram's own page: the connection, its token and whether this transport is
 * switched on under the master switch.
 *
 * The token is written through the channel's `configure`, because only Telegram
 * can say whether a token is usable. The bot is checked with `getMe` before the
 * value is stored, so a typo is reported at the field and an earlier working
 * token keeps working.
 */
export function TelegramSettingsPage({
  notify,
  store,
}: {
  notify: Notify;
  store: MessagingStore;
}) {
  const settings = store.status?.settings ?? null;
  const channel = store.status?.channels.find((entry) => entry.id === "telegram") ?? null;
  const [token, setToken] = useState("");
  const [savingToken, setSavingToken] = useState(false);

  if (!settings) return <CommunicationsLoading title="Telegram" error={store.error} />;

  const tokenSaved = channel?.config.tokenSaved === true;
  const identity = channel?.identity ?? null;
  const enabled = settings.channels.includes("telegram");
  const status = channel?.error
    ? channel.error
    : channel?.running
      ? "Listening for messages."
      : tokenSaved
        ? "Saved. It connects when both switches are on."
        : "No bot token yet.";

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

  return (
    <>
      <CommunicationsTitle title="Telegram" settings={settings} store={store} />
      <SettingsSection title="Connection">
        <SettingRow
          anchor="communications.telegram.enabled"
          label="Telegram"
          description={
            enabled
              ? "This transport is switched on. The master switch above gates it as well."
              : "Switched off. No polling happens until both this and the master switch are on."
          }
          control={
            <Toggle
              checked={enabled}
              onChange={(next) =>
                void saveSettings(
                  store,
                  settings,
                  {
                    channels: next
                      ? [...new Set([...settings.channels, "telegram"])]
                      : settings.channels.filter((id) => id !== "telegram"),
                  },
                  notify,
                )
              }
              label="Telegram transport"
            />
          }
        />

        <SettingRow
          anchor="communications.telegram.status"
          label={identity ? `Connected as ${identity}` : "Bot status"}
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
      </SettingsSection>
    </>
  );
}
