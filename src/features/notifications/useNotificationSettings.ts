import { useEffect, useState } from "react";
import {
  cachedNotificationSettings,
  DEFAULT_NOTIFICATIONS,
  loadNotificationSettings,
  subscribeNotificationSettings,
  type NotificationSettings,
} from "./settings";

/**
 * The notification record, shared by the settings pages and by anything that
 * plays the completion sound.
 *
 * Starts from the cache so a page opened after startup paints the stored values
 * on its first frame, and follows saves through the subscription so both pages
 * agree without either fetching again.
 */
export function useNotificationSettings(): NotificationSettings {
  const [settings, setSettings] = useState<NotificationSettings>(
    () => cachedNotificationSettings() ?? DEFAULT_NOTIFICATIONS,
  );

  useEffect(() => {
    const unsubscribe = subscribeNotificationSettings(setSettings);
    void loadNotificationSettings().catch(() => undefined);
    return unsubscribe;
  }, []);

  return settings;
}
