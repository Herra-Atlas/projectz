import type { TasteSettings, TasteStatus } from "./types";

/**
 * The settings half of a `TasteStatus`, from the stored record alone.
 *
 * The settings page cannot draw without a record, and the record is one key read. The
 * profile, the last run and the running flag are the page's *content* -- they belong to
 * the full read and arrive a moment later -- so the page is seeded with this instead of
 * waiting for all of them, which is what the "Reading settings…" placeholder used to be.
 *
 * **Seeded, never merged.** The caller only fills this in while it holds nothing; a real
 * status that has already arrived would be strictly better than this one, and replacing it
 * with an empty profile would blank the page for a moment.
 *
 * `null` means the key is absent -- a fresh install -- and the backend's defaults are the
 * honest answer there. They are not repeated here beyond what the record itself defines:
 * an absent `intervalDays` takes the backend's default cadence only in its own `Default`
 * impl, so it is left at the TypeScript default of the same value rather than invented.
 */
export function statusFrame(stored: TasteSettings | null): TasteStatus {
  const settings: TasteSettings = stored ?? {
    enabled: false,
    models: [],
    intervalDays: 4,
    nextRunAt: null,
  };
  return {
    enabled: settings.enabled,
    settings,
    items: [],
    document: "",
    lastRun: null,
    running: false,
    nextRunAt: settings.nextRunAt ?? null,
  };
}
