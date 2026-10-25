import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { AccessSet } from "../chat/access";
import type { MessagingSettings, MessagingStatus } from "./types";

/**
 * The messaging record and every channel's live state.
 *
 * # Why one hook owns it
 *
 * The master switch, the allowlist and each channel's token are one configuration
 * read and one page. Two hooks would mean two reads that can disagree about whether
 * messaging is on -- the same reason `useAssist` owns both its switch and its
 * status.
 *
 * # Why it is mounted by the modal rather than by the page
 *
 * The dialog outlives the page: it is open while the user is in another section.
 * Starting the read there means the Communications click lands on data instead of
 * on "Reading settings…".
 */
export type MessagingStore = ReturnType<typeof useMessaging>;

export function useMessaging(enabled: boolean) {
  const [status, setStatus] = useState<MessagingStatus | null>(null);
  const [hostAccess, setHostAccess] = useState<AccessSet | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const mounted = useRef(true);

  const refresh = useCallback(async () => {
    try {
      // One read for the communications record and one for the installation-wide
      // switches. They are independent records but one page reads both, and two
      // separate loading states would make Security flash.
      const [next, host] = await Promise.all([
        invoke<MessagingStatus>("messaging_status"),
        invoke<AccessSet>("host_access_get"),
      ]);
      if (mounted.current) {
        setStatus(next);
        setHostAccess(host);
        setError("");
      }
      return next;
    } catch (reason) {
      if (mounted.current) setError(String(reason));
      return null;
    }
  }, []);

  useEffect(() => {
    mounted.current = true;
    if (!enabled) return () => { mounted.current = false; };
    void refresh();
    // The tray has no switch for messaging, but a save from this page and a
    // conversation written by a channel both land here, so the page and the
    // sidebar read one event rather than polling each other.
    const unlisten = listen("messaging-event", () => { void refresh(); });
    return () => {
      mounted.current = false;
      void unlisten.then((off) => off());
    };
  }, [enabled, refresh]);

  /**
   * Writes the record, then settles on what the backend stored.
   *
   * The write is the truth: the allowlist is filtered to numeric ids and a
   * person with Agent off has their default mode forced to Chat, so a control
   * that shows what was asked for would eventually show something not stored.
   */
  const save = useCallback(async (next: MessagingSettings) => {
    setStatus((current) => (current ? { ...current, settings: next } : current));
    setBusy(true);
    try {
      await invoke<MessagingSettings>("messaging_save_settings", { settings: next });
      return await refresh();
    } catch (reason) {
      setError(String(reason));
      await refresh();
      return null;
    } finally {
      setBusy(false);
    }
  }, [refresh]);

  /** Moves the one master switch every channel reads. */
  const setEnabled = useCallback(async (next: boolean) => {
    setBusy(true);
    try {
      const updated = await invoke<MessagingStatus>("messaging_set_enabled", { enabled: next });
      if (mounted.current) setStatus(updated);
      return updated;
    } catch (reason) {
      setError(String(reason));
      return null;
    } finally {
      setBusy(false);
    }
  }, []);

  /**
   * Writes one channel's own configuration, validating it first.
   *
   * The backend refuses to store a token it could not reach Telegram with, so a
   * mistyped one leaves the previous credential working. The rejection is returned
   * rather than swallowed, so the page can put it beside the field.
   */
  const saveChannel = useCallback(async (channel: string, config: unknown) => {
    setBusy(true);
    try {
      const view = await invoke<unknown>("messaging_save_channel", { channel, config });
      await refresh();
      return view;
    } finally {
      setBusy(false);
    }
  }, [refresh]);

  /**
   * Writes the installation-wide switches.
   *
   * Saved on each toggle: unlike a text field there is no half-finished value,
   * and the next run reads this record when its gate is built.
   */
  const saveHostAccess = useCallback(async (access: AccessSet) => {
    const previous = hostAccess;
    setHostAccess(access);
    setBusy(true);
    try {
      const stored = await invoke<AccessSet>("host_access_save", { access });
      if (mounted.current) setHostAccess(stored);
      return stored;
    } catch (reason) {
      if (mounted.current) {
        setHostAccess(previous);
        setError(String(reason));
      }
      return null;
    } finally {
      setBusy(false);
    }
  }, [hostAccess]);

  return {
    status,
    hostAccess,
    error,
    busy,
    refresh,
    save,
    setEnabled,
    saveChannel,
    saveHostAccess,
  };
}
