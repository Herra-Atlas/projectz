import { invoke } from "@tauri-apps/api/core";

/**
 * The one record behind General settings.
 *
 * Read once per app run, at startup, and kept here rather than in each page that
 * needs a field. The chat page used to fetch it on mount, which is why the
 * greeting could paint its fallback for a moment before the stored name arrived:
 * the read was a round trip that began when the page appeared. Everything that
 * reads this record now shares one cache, so a page opened after startup draws
 * the stored name on its first frame.
 *
 * Writes go through `saveGeneralSettings`, which updates the cache before the
 * write lands. Settings is both the only writer and the only editor, so the
 * cache cannot fall behind an edit made elsewhere.
 */

/** The settings key the Rust side reads for instructions and notifications. */
export const GENERAL_SETTINGS_KEY = "app.general";

/** Where the pre-database build kept this record, read once to migrate it. */
const LEGACY_STORAGE_KEY = "projectz.general-settings";

export type GeneralSettings = {
  userName: string;
  instructions: string;
  responseNotifications: boolean;
  showMetrics: boolean;
  sendWith: "Enter" | "Ctrl + Enter";
};

export const DEFAULT_GENERAL: GeneralSettings = {
  userName: "",
  instructions: "",
  responseNotifications: false,
  showMetrics: true,
  sendWith: "Enter",
};

/**
 * Fills in defaults for any key the stored record predates.
 *
 * The name is trimmed here, once, so every reader gets it clean: a paste that
 * left a trailing space would otherwise become a space before a comma in the
 * greeting.
 */
export function withGeneralDefaults(
  saved: Partial<GeneralSettings> | null | undefined,
): GeneralSettings {
  return {
    ...DEFAULT_GENERAL,
    ...(saved ?? {}),
    userName: (saved?.userName ?? "").trim(),
  };
}

let cached: GeneralSettings | null = null;
let pending: Promise<GeneralSettings> | null = null;
const listeners = new Set<(settings: GeneralSettings) => void>();

/** The record if it has been read this run, otherwise `null`. */
export function cachedGeneralSettings(): GeneralSettings | null {
  return cached;
}

/** Listens for a save; returns the unsubscribe function a hook effect needs. */
export function subscribeGeneralSettings(
  listener: (settings: GeneralSettings) => void,
): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function publish(next: GeneralSettings) {
  cached = next;
  for (const listener of listeners) listener(next);
}

/** Reads the record, migrating the localStorage one when the database has none. */
async function readStored(): Promise<GeneralSettings> {
  const saved = await invoke<Partial<GeneralSettings> | null>("database_get_setting", {
    key: GENERAL_SETTINGS_KEY,
  });
  if (saved) return withGeneralDefaults(saved);

  // The legacy record is only ever adopted once; `chatRepository` already copies
  // it over on a first launch, so this covers the install that migrated chats but
  // not this record.
  try {
    const legacy = localStorage.getItem(LEGACY_STORAGE_KEY);
    if (legacy) {
      const migrated = withGeneralDefaults(JSON.parse(legacy) as Partial<GeneralSettings>);
      await invoke("database_set_setting", { key: GENERAL_SETTINGS_KEY, value: migrated });
      localStorage.removeItem(LEGACY_STORAGE_KEY);
      return migrated;
    }
  } catch {
    // An unreadable legacy record is not worth failing startup over; the defaults
    // below are what a fresh install has.
  }
  return { ...DEFAULT_GENERAL };
}

/**
 * Reads the record once per run.
 *
 * Concurrent callers share one request. A failure is **not** cached, so a call
 * made before the database is open (the settings modal mounting early, a page
 * rendered outside startup) cannot leave defaults behind for the rest of the
 * run; the next call tries again.
 */
export function loadGeneralSettings(): Promise<GeneralSettings> {
  if (cached) return Promise.resolve(cached);
  if (pending) return pending;
  pending = readStored().then(
    (settings) => {
      pending = null;
      publish(settings);
      return settings;
    },
    (error: unknown) => {
      pending = null;
      throw error;
    },
  );
  return pending;
}

/**
 * Saves the record and updates the cache at once.
 *
 * The cache is published before the write completes so the control the user just
 * touched does not wait on SQLite; a failed write is the caller's to report.
 */
export async function saveGeneralSettings(next: GeneralSettings): Promise<void> {
  publish(next);
  await invoke("database_set_setting", { key: GENERAL_SETTINGS_KEY, value: next });
}
