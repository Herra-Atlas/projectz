import { AccessSwitches } from "../AccessSwitches";
import { SettingRow, SettingsSection } from "../SettingsSection";
import type { AccessSet } from "../../features/chat/access";
import type { MessagingUser } from "../../features/communications/types";
import type { MessagingStore } from "../../features/communications/useMessaging";
import type { Notify } from "../../features/notifications/types";
import { AllowlistEditor } from "./AllowlistEditor";
import { TelegramUserAccess } from "./TelegramUserAccess";
import {
  CommunicationsLoading,
  CommunicationsTitle,
  saveSettings,
} from "./shared";

/**
 * Who may reach the agent, and what any run on this installation may touch.
 *
 * These are two different boundaries. The allowlist decides *who* a channel obeys;
 * the host switches decide *what* any run -- app, job, sub-agent or phone -- may
 * use. Both are here because a reader asking "is this safe" is asking both.
 */
export function CommunicationSecurityPage({
  notify,
  store,
}: {
  notify: Notify;
  store: MessagingStore;
}) {
  const settings = store.status?.settings ?? null;
  if (!settings) {
    return <CommunicationsLoading title="Communications security" error={store.error} />;
  }

  const saveHost = (access: AccessSet) => {
    void store.saveHostAccess(access).then((stored) => {
      if (!stored) notify("error", "The installation limit could not be saved.");
    });
  };

  const saveUsers = (users: Record<string, MessagingUser>) => {
    void saveSettings(store, settings, { users }, notify);
  };

  return (
    <>
      <CommunicationsTitle
        title="Communications security"
        settings={settings}
        store={store}
      />

      <SettingsSection title="Who may message">
        <SettingRow
          anchor="communications.telegram.allowed"
          label="Allowed Telegram user ids"
          description="Empty means nobody. The bot explains itself to a stranger only while this list is empty, and replies with the numeric id to add."
          stacked
          control={
            <AllowlistEditor
              ids={settings.allowedUserIds}
              onChange={(allowedUserIds) =>
                void saveSettings(store, settings, { allowedUserIds }, notify)
              }
              notify={notify}
            />
          }
          hint="A username can change hands; a numeric id cannot. Message the bot from the phone you want to use to learn its id."
        />

        <SettingRow
          anchor="communications.profiles"
          label="Per-person profiles"
          description="Each allowed user can send `/profile <text>` in the chat to set how answers should be written for them, and `/profile clear` to remove it."
          hint="Saved in the app data folder under telegram/<id>/profile.md, beside that person's workspace. It can change length, tone and what answers include, but not formatting or access."
        />
      </SettingsSection>

      <SettingsSection title="Per-person access">
        <SettingRow
          anchor="communications.userAccess"
          label="What each Telegram id may use"
          description="Each allowed id has its own Chat/Agent setting, default mode, permission and tool switches. Changes apply to that person's next message."
          stacked
          control={<TelegramUserAccess settings={settings} onChange={saveUsers} />}
          hint="This is the permission you were missing: disabling terminal for one id does not affect another id or the app chat."
        />
      </SettingsSection>

      <SettingsSection title="Installation limit">
        <SettingRow
          anchor="communications.hostAccess"
          label="What this installation may use"
          description="The hard outer limit for every run: app chats, jobs, sub-agents and Telegram. A tool off here is hidden everywhere, whatever a person or job asks for."
          stacked
          control={
            store.hostAccess ? (
              <AccessSwitches
                access={store.hostAccess}
                onChange={saveHost}
                labelPrefix="Host "
              />
            ) : (
              <p className="text-[12px] text-[var(--quiet)]">Reading the installation limit…</p>
            )
          }
          hint="A refusal names this record: “Host has disabled terminal for this account.” A per-person switch can remove more, never add it back."
        />
      </SettingsSection>
    </>
  );
}
