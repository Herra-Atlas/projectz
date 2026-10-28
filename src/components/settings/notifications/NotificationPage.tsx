import type { Notify } from "../../../features/notifications/types";
import {
  saveNotificationSettings,
  type AlertKind,
  type NotificationAlert,
  type NotificationSettings,
} from "../../../features/notifications/settings";
import { useNotificationSettings } from "../../../features/notifications/useNotificationSettings";
import { SettingRow, SettingsPageTitle, SettingsSection, Toggle } from "../../SettingsSection";
import { SoundPicker } from "./SoundPicker";

/**
 * The rows, in the order they are worth reading.
 *
 * One table rather than four hand-written blocks: every row is the same shape --
 * a switch and a sound -- so the only thing that should differ is the sentence
 * explaining which event it is about.
 */
const ALERTS: { kind: AlertKind; anchor: string; label: string; description: string }[] = [
  {
    kind: "approval",
    anchor: "notifications.approval",
    label: "Approval needed",
    description: "An agent is waiting for Allow or Deny on a tool call.",
  },
  {
    kind: "input",
    anchor: "notifications.input",
    label: "Input needed",
    description: "An agent has asked you a question and is waiting for the answer.",
  },
  {
    kind: "response",
    anchor: "notifications.response",
    label: "Response ready",
    description: "A reply finished in a conversation you were not looking at.",
  },
  {
    kind: "failed",
    anchor: "notifications.failed",
    label: "Turn failed",
    description: "A reply stopped with an error in a conversation you were not looking at.",
  },
];

/**
 * One page for every alert: a switch and a sound beside it.
 *
 * **The sound is the notification.** There is no system toast and no second
 * list of sounds to keep in step: each category owns its own tone, so a failure
 * and a finished reply can be told apart without reading anything.
 */
export function NotificationPage({ notify }: { notify: Notify }) {
  const settings = useNotificationSettings();

  const save = (next: NotificationSettings) => {
    void saveNotificationSettings(next).catch((reason: unknown) =>
      notify("error", `Notifications could not be saved: ${String(reason)}`),
    );
  };

  /** Replaces one category, leaving the other three exactly as they were. */
  const updateAlert = (kind: AlertKind, patch: Partial<NotificationAlert>) => {
    save({
      ...settings,
      alerts: { ...settings.alerts, [kind]: { ...settings.alerts[kind], ...patch } },
    });
  };

  return (
    <>
      <SettingsPageTitle>
        <h2 className="text-[17px] font-semibold tracking-tight">Notifications</h2>
      </SettingsPageTitle>

      <SettingsSection title="Alerts">
        <SettingRow
          anchor="notifications.enabled"
          label="Enable notifications"
          description="Sounds played by ProjectZ itself when one of the events below happens. Off silences every category without forgetting its sound."
          control={
            <Toggle
              label="Enable notifications"
              checked={settings.enabled}
              onChange={(enabled) => save({ ...settings, enabled })}
            />
          }
        />
        {ALERTS.map((row) => {
          const alert = settings.alerts[row.kind];
          return (
            <SettingRow
              key={row.kind}
              anchor={row.anchor}
              label={row.label}
              description={row.description}
              control={
                <div className="flex items-center gap-3">
                  <SoundPicker
                    label={`${row.label} sound`}
                    value={alert.sound}
                    onChange={(sound) => updateAlert(row.kind, { sound })}
                  />
                  <Toggle
                    label={`Play a sound when ${row.label.toLowerCase()}`}
                    checked={alert.enabled}
                    onChange={(enabled) => updateAlert(row.kind, { enabled })}
                  />
                </div>
              }
            />
          );
        })}
      </SettingsSection>
    </>
  );
}
