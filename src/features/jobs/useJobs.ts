import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Job, JobEvent, JobRun, NewJob, SchedulerStatus } from "./types";

/**
 * The job list and the mutations that change it.
 *
 * Reads once when the screen opens and again on every change, rather than holding
 * a copy of every job in state and patching it: the scheduler is writing these rows
 * in the background, so the database is the only copy that is ever right.
 */
export function useJobs(active: boolean) {
  const [jobs, setJobs] = useState<Job[]>([]);
  const [loading, setLoading] = useState(false);

  const reload = useCallback(async () => {
    setLoading(true);
    try {
      setJobs(await invoke<Job[]>("job_list"));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (active) void reload();
  }, [active, reload]);

  const create = useCallback(
    async (job: NewJob) => {
      await invoke<Job>("job_create", { job });
      await reload();
    },
    [reload],
  );

  const update = useCallback(
    async (id: string, job: NewJob) => {
      await invoke<Job>("job_update", { id, job });
      await reload();
    },
    [reload],
  );

  const remove = useCallback(
    async (id: string) => {
      await invoke("job_delete", { id });
      await reload();
    },
    [reload],
  );

  const setEnabled = useCallback(
    async (id: string, enabled: boolean) => {
      await invoke("job_set_enabled", { id, enabled });
      await reload();
    },
    [reload],
  );

  const runNow = useCallback(
    async (id: string) => {
      await invoke("job_run_now", { id });
      await reload();
    },
    [reload],
  );

  return { jobs, loading, reload, create, update, remove, setEnabled, runNow };
}

/** Total RAM and VRAM, so a percentage limit can be shown as the bytes it means. */
export type MachineTotals = {
  ram_total_bytes: number | null;
  vram_total_bytes: number | null;
};

/**
 * The machine's totals, read once.
 *
 * They do not change while the app is open, so there is nothing to poll: the read
 * happens when the editor mounts and the value is kept.
 */
export function useMachineTotals(): MachineTotals {
  const [totals, setTotals] = useState<MachineTotals>({ ram_total_bytes: null, vram_total_bytes: null });
  useEffect(() => {
    let live = true;
    void invoke<MachineTotals>("job_machine_totals")
      .then((read) => { if (live) setTotals(read); })
      // A machine with no readable figure is a normal state, not an error to show:
      // the sliders read "unknown" rather than the page refusing to open.
      .catch(() => undefined);
    return () => { live = false; };
  }, []);
  return totals;
}

/** One job's recent attempts. */
export function useJobRuns(jobId: string | null, reloadKey = 0) {
  const [runs, setRuns] = useState<JobRun[]>([]);

  const reload = useCallback(async () => {
    if (!jobId) {
      setRuns([]);
      return;
    }
    setRuns(await invoke<JobRun[]>("job_runs", { id: jobId }));
  }, [jobId]);

  useEffect(() => {
    void reload();
  }, [reload, reloadKey]);

  return { runs, reload };
}

/**
 * The scheduler's state, kept live by its events.
 *
 * The event carries a status rather than the whole job list, so it is used only as
 * a signal to re-read. That keeps one source of truth: the database, which the
 * scheduler also writes.
 */
export function useScheduler(onActivity: () => void) {
  const [status, setStatus] = useState<SchedulerStatus>({ paused: false, running: [] });

  const reload = useCallback(async () => {
    setStatus(await invoke<SchedulerStatus>("job_scheduler_status"));
  }, []);

  useEffect(() => {
    let live = true;
    void reload();
    const pending = listen<JobEvent>("job-event", () => {
      if (!live) return;
      void reload();
      onActivity();
    });
    return () => {
      live = false;
      pending.then((unlisten) => unlisten());
    };
    // `onActivity` is deliberately not a dependency: callers pass an inline arrow,
    // and re-subscribing on every render would drop events between teardown and
    // the next listen.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [reload]);

  const setPaused = useCallback(
    async (paused: boolean) => {
      await invoke("job_scheduler_pause", { paused });
      await reload();
    },
    [reload],
  );

  return { status, reload, setPaused };
}
