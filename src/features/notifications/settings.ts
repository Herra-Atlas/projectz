import { invoke } from "@tauri-apps/api/core";

/**
 * The one record behind the Notifications page.
 *
 * **Sounds are the notification.** The app no longer asks Windows to show a
 * toast; an alert is a tone played inside ProjectZ, which is the only thing the
 * category switches here can promise. One record rather than a key per toggle so
 * the master switch and the four categories are read together, and so the sound
 * a category carries cannot drift from the category itself.
 */

/** The settings key. Frontend-only: nothing in the backend reads this record. */
export const NOTIFICATION_SETTINGS_KEY = "app.notifications";

/** The four things worth making a sound about. */
export const ALERT_KINDS = ["approval", "input", "response", "failed"] as const;
export type AlertKind = (typeof ALERT_KINDS)[number];

/** Every tone the picker offers, and the names it draws for them. */
export const NOTIFICATION_SOUNDS = ["chime", "soft", "pop", "bell", "wood", "pulse"] as const;
export type NotificationSound = (typeof NOTIFICATION_SOUNDS)[number];

export const SOUND_LABELS: Record<NotificationSound, string> = {
  chime: "Chime",
  soft: "Soft",
  pop: "Pop",
  bell: "Bell",
  wood: "Wood",
  pulse: "Pulse",
};

/** One category: whether it alerts, and how it sounds. */
export type NotificationAlert = { enabled: boolean; sound: NotificationSound };

export type NotificationSettings = {
  /** The master switch: off means no sound from any category. */
  enabled: boolean;
  alerts: Record<AlertKind, NotificationAlert>;
};

/**
 * The defaults: what the app did before this page, expressed in sound.
 *
 * Failure and approval are on, completion is off, and each category starts with a
 * different tone so a user who never opens the page can still tell them apart.
 */
export const DEFAULT_NOTIFICATIONS: NotificationSettings = {
  enabled: true,
  alerts: {
    approval: { enabled: true, sound: "chime" },
    input: { enabled: true, sound: "soft" },
    response: { enabled: false, sound: "pop" },
    failed: { enabled: true, sound: "bell" },
  },
};

/**
 * Fills in defaults per category, keeping every sound inside the list.
 *
 * A record written before a category or a sound existed must still read: a
 * missing switch takes its default rather than making the whole record
 * unreadable, and a hand-edited sound name is replaced rather than reaching the
 * player, which would silently play nothing.
 */
export function withNotificationDefaults(
  saved: Partial<NotificationSettings> | null | undefined,
): NotificationSettings {
  const alerts = { ...DEFAULT_NOTIFICATIONS.alerts };
  for (const kind of ALERT_KINDS) {
    const stored = saved?.alerts?.[kind];
    const sound = stored?.sound;
    alerts[kind] = {
      enabled:
        typeof stored?.enabled === "boolean"
          ? stored.enabled
          : DEFAULT_NOTIFICATIONS.alerts[kind].enabled,
      sound: NOTIFICATION_SOUNDS.includes(sound as NotificationSound)
        ? (sound as NotificationSound)
        : DEFAULT_NOTIFICATIONS.alerts[kind].sound,
    };
  }
  return { enabled: saved?.enabled !== false, alerts };
}

/**
 * The sound a category should make, or `null` when it is off.
 *
 * One question asked by both the page and the chat view, so a category switched
 * off cannot still make a noise and a master switched off cannot be escaped.
 */
export function soundFor(
  settings: NotificationSettings,
  kind: AlertKind,
): NotificationSound | null {
  if (!settings.enabled) return null;
  const alert = settings.alerts[kind];
  return alert.enabled ? alert.sound : null;
}

let cached: NotificationSettings | null = null;
let pending: Promise<NotificationSettings> | null = null;
const listeners = new Set<(settings: NotificationSettings) => void>();

/** The record if it has been read this run, otherwise `null`. */
export function cachedNotificationSettings(): NotificationSettings | null {
  return cached;
}

/** Listens for a save; returns the unsubscribe a hook effect needs. */
export function subscribeNotificationSettings(
  listener: (settings: NotificationSettings) => void,
): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function publish(next: NotificationSettings) {
  cached = next;
  for (const listener of listeners) listener(next);
}

/** Reads the record once per run; concurrent callers share one request. */
export function loadNotificationSettings(): Promise<NotificationSettings> {
  if (cached) return Promise.resolve(cached);
  if (pending) return pending;
  pending = invoke<Partial<NotificationSettings> | null>("database_get_setting", {
    key: NOTIFICATION_SETTINGS_KEY,
  })
    .then((saved) => withNotificationDefaults(saved))
    .then(
      (settings) => {
        pending = null;
        publish(settings);
        return settings;
      },
      (error: unknown) => {
        // A failure is not cached: a read made before the database is open must
        // not leave defaults behind for the rest of the run.
        pending = null;
        throw error;
      },
    );
  return pending;
}

/**
 * Saves the record and updates the cache at once, so the switch the user touched
 * does not wait on SQLite. A failed write is the caller's to report.
 */
export async function saveNotificationSettings(next: NotificationSettings): Promise<void> {
  publish(next);
  await invoke("database_set_setting", { key: NOTIFICATION_SETTINGS_KEY, value: next });
}
