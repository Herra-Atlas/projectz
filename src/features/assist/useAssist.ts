import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { AssistEvent, AssistSettings, AssistSettingsView, AssistStatus } from "./types";

/**
 * The assist settings, and their live status.
 *
 * # Why one hook owns both
 *
 * The switch, the model and the thresholds are read and written through the same record, and
 * the tray is a second door into the switch. Two hooks would mean two reads that can disagree
 * about whether assist is on; one read, refreshed whenever the backend says the switch moved,
 * cannot.
 *
 * # Why a save is optimistic and then re-read
 *
 * The controls hold the value the user just chose, so a round trip before the toggle moves
 * would feel broken. The write is the truth, though -- the backend clamps every threshold --
 * so the status is re-read afterwards and the control settles on what was actually stored
 * rather than on what was asked for. A user who types 0 into a field with a floor of 4 sees 4,
 * which is better than seeing 0 and wondering why nothing happens.
 */

/**
 * What `useAssist` answers.
 *
 * Named so the settings modal can own the hook and hand the page this object: the modal
 * outlives the page (the dialog is open, the page is merely selected), so a read started
 * there happens while the user is still looking at another section, and the page opens on
 * data rather than on a placeholder.
 */
export type AssistStore = ReturnType<typeof useAssist>;

/** The assist settings, and their live status -- see the type for why it is one hook. */
export function useAssist(enabled: boolean) {
  const [status, setStatus] = useState<AssistStatus | null>(null);
  const [error, setError] = useState("");
  /** Whether a save or the initial read is in flight, for the page's own use. */
  const [busy, setBusy] = useState(false);
  const mounted = useRef(true);

  const refresh = useCallback(async () => {
    try {
      const next = await invoke<AssistStatus>("assist_status");
      if (mounted.current) {
        setStatus(next);
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
    // Fast path first: the record and its bounds are one settings read, while the
    // full status also reads the bridge and the hour's spending. Seeding the page
    // with this means the controls appear as soon as the record is known; the full
    // status replaces it with the live facts. The page no longer *depends* on the
    // full status -- it draws without it -- so this is what the first paint waits
    // for, and it is deliberately the cheapest read there is.
    void invoke<AssistSettingsView>("assist_settings")
      .then((view) => {
        if (!mounted.current) return;
        setStatus((current) => current ?? {
          enabled: view.settings.enabled,
          dryRun: view.settings.dryRun,
          settings: view.settings,
          model: null,
          // Not drawn, but part of the wire shape, and the full status fills it.
          modelReady: null,
          suggestedSubjects: view.suggestedSubjects,
          // The socket starts with the app, so "running" is the honest default
          // while the full status is on its way; only a real failure or a missing
          // port will change it.
          bridge: { running: true, port: null, error: null, clients: [] },
          callsPastHour: 0,
          levelBounds: view.levelBounds,
        });
      })
      .catch(() => undefined);
    void refresh();
    // The switch arrives here, so a change made in the tray or the extension's popup shows up
    // in this window without the two polling each other. The record is re-read rather than
    // patched onto the status because the record is the backend's to clamp.
    const unlisten = listen<AssistEvent>("assist-event", (event) => {
      if (event.payload.kind === "state") void refresh();
    });
    return () => {
      mounted.current = false;
      void unlisten.then((off) => off());
    };
  }, [enabled, refresh]);

  /**
   * Writes the record and settles the page on what was stored.
   *
   * Returns the re-read status so a caller can see a clamped value immediately rather than
   * waiting a render for it.
   */
  const save = useCallback(async (next: AssistSettings) => {
    setStatus((current) => (current ? { ...current, settings: next, enabled: next.enabled } : current));
    setBusy(true);
    try {
      await invoke<AssistSettings>("assist_save_settings", { settings: next });
      return await refresh();
    } catch (reason) {
      setError(String(reason));
      // Re-read rather than leaving the optimistic value: a write that failed means the screen
      // is showing something that is not stored.
      await refresh();
      return null;
    } finally {
      setBusy(false);
    }
  }, [refresh]);

  /** Moves the switch, which is the same call the tray's item makes. */
  const setEnabled = useCallback(async (next: boolean) => {
    setBusy(true);
    try {
      const updated = await invoke<AssistStatus>("assist_set_enabled", { enabled: next });
      if (mounted.current) setStatus(updated);
      return updated;
    } catch (reason) {
      setError(String(reason));
      return null;
    } finally {
      setBusy(false);
    }
  }, []);

  return { status, error, busy, refresh, save, setEnabled };
}
