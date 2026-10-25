import {
  ANCHOR_ATTRIBUTE,
  FlashHighlight,
  MARKABLE_ATTRIBUTE,
  SettingsPageTitle,
  Toggle,
} from "../SettingsSection";
import type {
  MessagingSettings,
  MessagingStatus,
} from "../../features/communications/types";
import type { MessagingStore } from "../../features/communications/useMessaging";
import type { Notify } from "../../features/notifications/types";

/**
 * Saves one patch of the shared messaging record, then settles on what was stored.
 *
 * The write is the truth: the allowlist is filtered, instructions are trimmed and
 * a person with Agent off has their default mode forced to Chat, so a control that
 * showed what was asked for would eventually show something the backend does not
 * hold.
 */
export async function saveSettings(
  store: MessagingStore,
  settings: MessagingSettings,
  patch: Partial<MessagingSettings>,
  notify: Notify,
): Promise<MessagingStatus | null> {
  const next = await store.save({ ...settings, ...patch });
  if (!next) notify("error", "That could not be saved.");
  return next;
}

/** The title band every Communications page shares, including the master switch. */
export function CommunicationsTitle({
  title,
  settings,
  store,
}: {
  title: string;
  settings: MessagingSettings;
  store: MessagingStore;
}) {
  return (
    <SettingsPageTitle>
      <h2 className="text-[17px] font-semibold tracking-tight">{title}</h2>
      <div
        className="flex items-center gap-2.5"
        {...{
          [ANCHOR_ATTRIBUTE]: "communications.enabled",
          [MARKABLE_ATTRIBUTE]: "",
        }}
      >
        <FlashHighlight anchor="communications.enabled" />
        <span className="text-xs text-[var(--muted)]">
          {settings.enabled ? "Messaging on" : "Messaging off"}
        </span>
        <Toggle
          checked={settings.enabled}
          onChange={(next) => void store.setEnabled(next)}
          label="Messaging"
        />
      </div>
    </SettingsPageTitle>
  );
}

/** The page body while the record has not arrived, or after it failed. */
export function CommunicationsLoading({
  title,
  error,
}: {
  title: string;
  error: string;
}) {
  return (
    <>
      <SettingsPageTitle>
        <h2 className="text-[17px] font-semibold tracking-tight">{title}</h2>
      </SettingsPageTitle>
      <p className="px-4 py-6 text-sm text-[var(--muted)]">
        {error || "Reading settings…"}
      </p>
    </>
  );
}
