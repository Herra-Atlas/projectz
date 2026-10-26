import { useEffect, useState } from "react";
import {
  cachedGeneralSettings,
  DEFAULT_GENERAL,
  loadGeneralSettings,
  subscribeGeneralSettings,
  type GeneralSettings,
} from "./generalSettings";

/**
 * The General settings record, shared by every page that needs it.
 *
 * The initial state comes from the store synchronously, so a component mounted
 * after startup paints the stored values on its first frame instead of a
 * placeholder. Startup primes the store before the app renders, which is what
 * removes the greeting's fallback flash; a component mounted without that prime
 * asks for the read itself, and every later mount is served from the cache.
 *
 * Saves arrive here through the subscription rather than a page re-reading the
 * record, so editing your name in Settings updates the greeting without either
 * side fetching again.
 */
export function useGeneralSettings(): GeneralSettings {
  const [settings, setSettings] = useState<GeneralSettings>(
    () => cachedGeneralSettings() ?? DEFAULT_GENERAL,
  );

  useEffect(() => {
    const unsubscribe = subscribeGeneralSettings(setSettings);
    // Served from the cache in the normal case; the read is started rather than
    // awaited because the cache is the value this page already has.
    void loadGeneralSettings().catch(() => undefined);
    return unsubscribe;
  }, []);

  return settings;
}
