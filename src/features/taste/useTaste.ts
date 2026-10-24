import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { statusFrame } from "./statusFrame";
import type { TasteEvent, TasteRun, TasteSettings, TasteStatus } from "./types";

/** How many runs the settings page keeps in its history. */
const HISTORY_LIMIT = 20;

/**
 * What `useTaste` answers.
 *
 * Named so the settings modal can own the hook and hand the page this object -- the
 * dialog outlives the page, so the read starts when the window opens rather than when
 * the section is clicked.
 */
export type TasteStore = ReturnType<typeof useTaste>;

/**
 * The tasting record, its profile and its history.
 *
 * # Why one hook owns all three
 *
 * A run changes the settings (its next time), the profile and the history at once.
 * Three hooks would be three reads that can disagree about whether a run is
 * happening, which is exactly the state the page must not show. One refresh after
 * every event keeps them describing the same moment.
 *
 * # Why a save is optimistic and then re-read
 *
 * The toggle must move under the pointer, but the backend clamps the interval and
 * moves the next run when the cadence changes. The write is the truth, so the
 * record is re-read after it and the control settles on what was actually stored.
 */
export function useTaste(enabled: boolean) {
  const [status, setStatus] = useState<TasteStatus | null>(null);
  const [history, setHistory] = useState<TasteRun[]>([]);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const mounted = useRef(true);

  const refresh = useCallback(async () => {
    try {
      const next = await invoke<TasteStatus>("taste_status");
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

  const loadHistory = useCallback(async () => {
    try {
      const runs = await invoke<TasteRun[]>("taste_runs", { limit: HISTORY_LIMIT });
      if (mounted.current) setHistory(runs);
      return runs;
    } catch (reason) {
      if (mounted.current) setError(String(reason));
      return [];
    }
  }, []);

  useEffect(() => {
    mounted.current = true;
    if (!enabled) return () => { mounted.current = false; };
    // Fast path first. The settings record is the one thing the page cannot draw
    // without, and it is a single key read; the profile and the last run are the
    // page's content and can arrive a moment later. Without this the first paint
    // waited for all three, which is what made this page feel slower than the rest.
    void invoke<TasteSettings>("database_get_setting", { key: "app.taste" })
      .then((stored) => {
        if (!mounted.current) return;
        setStatus((current) => current ?? statusFrame(stored));
      })
      .catch(() => undefined);
    void refresh();
    void loadHistory();
    // A run is a background event: the page has to learn about its start and end
    // without polling. Both reads are cheap and neither touches a model.
    const unlisten = listen<TasteEvent>("taste-event", () => {
      void refresh();
      void loadHistory();
    });
    return () => {
      mounted.current = false;
      void unlisten.then((off) => off());
    };
  }, [enabled, refresh, loadHistory]);

  /** Writes the record and settles on what the backend stored. */
  const save = useCallback(async (next: TasteSettings) => {
    setStatus((current) => (current ? { ...current, settings: next, enabled: next.enabled } : current));
    setBusy(true);
    try {
      const stored = await invoke<TasteSettings>("taste_save_settings", { settings: next });
      await refresh();
      return stored;
    } catch (reason) {
      setError(String(reason));
      await refresh();
      return null;
    } finally {
      setBusy(false);
    }
  }, [refresh]);

  /** Runs a pass now. Returns the stored run so the caller can report its status. */
  const runNow = useCallback(async () => {
    setBusy(true);
    try {
      const run = await invoke<TasteRun>("taste_run_now");
      await refresh();
      await loadHistory();
      return run;
    } catch (reason) {
      setError(String(reason));
      await refresh();
      return null;
    } finally {
      setBusy(false);
    }
  }, [refresh, loadHistory]);

  /** Rewrites one fact. Rejects so an open editor can stay open on a refusal. */
  const updateItem = useCallback(async (id: number, text: string) => {
    try {
      const next = await invoke<TasteStatus>("taste_update_item", { id, text });
      if (mounted.current) setStatus(next);
      return next;
    } catch (reason) {
      setError(String(reason));
      throw reason;
    }
  }, []);

  /** Deletes one fact. */
  const deleteItem = useCallback(async (id: number) => {
    try {
      const next = await invoke<TasteStatus>("taste_delete_item", { id });
      if (mounted.current) setStatus(next);
      return next;
    } catch (reason) {
      setError(String(reason));
      throw reason;
    }
  }, []);

  return {
    status,
    history,
    error,
    busy,
    refresh,
    loadHistory,
    save,
    runNow,
    updateItem,
    deleteItem,
  };
}
