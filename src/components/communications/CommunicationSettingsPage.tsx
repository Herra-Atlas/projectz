import { useEffect, useState } from "react";

import { SettingRow, SettingsSection, Toggle } from "../SettingsSection";
import type { MessagingSettings } from "../../features/communications/types";
import type { MessagingStore } from "../../features/communications/useMessaging";
import type { Notify } from "../../features/notifications/types";
import {
  CommunicationsLoading,
  CommunicationsTitle,
  saveSettings,
} from "./shared";

/**
 * Channel-wide behaviour: files, local models and the wording used for every
 * Telegram reply.
 *
 * Per-person modes and tools live in Security, next to the numeric identity they
 * belong to. Keeping them out of here is what stops a channel-wide switch from
 * being mistaken for a user permission.
 */
export function CommunicationSettingsPage({
  notify,
  store,
}: {
  notify: Notify;
  store: MessagingStore;
}) {
  const settings = store.status?.settings ?? null;
  const [instructions, setInstructions] = useState(settings?.instructions ?? "");
  const [savingInstructions, setSavingInstructions] = useState(false);

  useEffect(() => {
    setInstructions(settings?.instructions ?? "");
  }, [settings?.instructions]);

  if (!settings) {
    return <CommunicationsLoading title="Communications settings" error={store.error} />;
  }

  const commit = (patch: Partial<MessagingSettings>) =>
    void saveSettings(store, settings, patch, notify);

  const saveInstructions = async () => {
    setSavingInstructions(true);
    try {
      const next = await saveSettings(store, settings, { instructions }, notify);
      if (next) notify("success", "Communication instructions saved.");
    } finally {
      setSavingInstructions(false);
    }
  };

  return (
    <>
      <CommunicationsTitle
        title="Communications settings"
        settings={settings}
        store={store}
      />

      <SettingsSection title="Channel">
        <SettingRow
          anchor="communications.files"
          label="File exchange"
          description={
            settings.allowFiles
              ? "Images, documents and text files can be sent in, and changed files are sent back."
              : "Both directions are refused. The bot will not download or upload attachments."
          }
          control={
            <Toggle
              checked={settings.allowFiles}
              onChange={(allowFiles) => commit({ allowFiles })}
              label="File exchange"
            />
          }
          hint="Files are stored in each sender's own app-data workspace. Turning this off does not remove files already there."
        />

      </SettingsSection>

      <SettingsSection title="Answer style">
        <SettingRow
          anchor="communications.instructions"
          label="Communication instructions"
          description="Used for every Telegram reply. When this is not empty it replaces General → Instructions for Telegram; each person can add their own style with /profile."
          stacked
          control={
            <div className="flex flex-col gap-2">
              <textarea
                value={instructions}
                onChange={(event) => setInstructions(event.target.value)}
                rows={6}
                maxLength={8000}
                placeholder="For example: keep answers under five sentences and ask before giving a long explanation."
                className="w-full rounded-lg border border-[var(--line)] bg-[var(--panel)] px-3 py-2 text-[13px] leading-5 text-[var(--text)] outline-none transition-colors focus:border-[var(--accent)]"
              />
              <div className="flex items-center justify-between gap-2">
                <span className="text-[11px] text-[var(--quiet)]">
                  Plain text is required here regardless; the transport does not render Markdown.
                </span>
                <button
                  type="button"
                  onClick={() => void saveInstructions()}
                  disabled={savingInstructions || instructions === settings.instructions}
                  className="min-h-8 rounded-md bg-[var(--accent)] px-3 text-[12px] font-medium text-[var(--accent-ink)] disabled:opacity-40"
                >
                  {savingInstructions ? "Saving…" : "Save"}
                </button>
              </div>
            </div>
          }
        />
      </SettingsSection>
    </>
  );
}
