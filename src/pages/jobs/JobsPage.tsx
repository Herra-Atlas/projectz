import { Fragment, useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Check, Clock, Pause, Play, Plus, RefreshCw, Trash2 } from "lucide-react";
import { useJobRuns, useJobs, useScheduler } from "../../features/jobs/useJobs";
import { Toggle } from "../../components/SettingsSection";
import type { Job, JobRun, JobSchedule, NewJob } from "../../features/jobs/types";
import type { Notify } from "../../features/notifications/types";
import JobEditor from "./JobEditor";

/**
 * The Jobs screen: what is scheduled, what it is doing, and what it has done.
 *
 * The list is the default view and the editor replaces it, the same two-state shape
 * the Files and Local models screens use -- only one of the two is ever useful at
 * once, and a job being edited should not be competing with twenty rows for the
 * reader's attention.
 */
export default function JobsPage({ notify, onOpenSession }: { notify: Notify; onOpenSession: (sessionId: string) => void }) {
  const { jobs, loading, reload, create, update, remove, setEnabled, runNow } = useJobs(true);
  const { status, setPaused } = useScheduler(() => void reload());
  const [editing, setEditing] = useState<{ job: Job | null } | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);

  // Follow the list: a job that was deleted cannot stay selected.
  useEffect(() => {
    if (selectedId && !jobs.some((job) => job.id === selectedId)) setSelectedId(null);
  }, [jobs, selectedId]);

  const selected = jobs.find((job) => job.id === selectedId) ?? null;

  /**
   * The list, arranged so the answer to "what is happening" is at the top.
   *
   * Grouped by lane -- a lane is what decides who waits for whom, so two jobs in one
   * are read together -- and the lanes themselves ordered by how alive they are.
   * Within a lane: the run in flight, then whatever is due soonest, then what is
   * switched off, and the jobs that are over at the bottom.
   */
  const running = new Set(status.running);
  const byLane = new Map<string, Job[]>();
  for (const job of jobs) byLane.set(job.lane, [...(byLane.get(job.lane) ?? []), job]);
  const live = (lane: string) => {
    const list = byLane.get(lane) ?? [];
    if (list.some((job) => running.has(job.id))) return 0;
    if (list.some((job) => !finished(job))) return 1;
    return 2;
  };
  const laneOrder = [...byLane.keys()].sort((a, b) => live(a) - live(b) || a.localeCompare(b));
  const ordered = [...jobs].sort((a, b) => {
    return (
      laneOrder.indexOf(a.lane) - laneOrder.indexOf(b.lane) ||
      rank(a, running.has(a.id)) - rank(b, running.has(b.id)) ||
      // Soonest first, and a job with no time at all last within its rank.
      (a.next_run_at ?? "9999").localeCompare(b.next_run_at ?? "9999")
    );
  });

  const submit = async (draft: NewJob) => {
    if (editing?.job) await update(editing.job.id, draft);
    else await create(draft);
    setEditing(null);
    notify("success", editing?.job ? "Job updated" : "Job created");
  };

  /**
   * Opens a job, which means opening what it last did.
   *
   * A reader who clicks a job wants its transcript, not a card about it. The newest
   * run that produced a session is the one to show; when there is none yet, the
   * history opens instead so the click still lands somewhere visible.
   */
  const openLatest = useCallback(
    async (job: Job) => {
      try {
        const runs = await invoke<JobRun[]>("job_runs", { id: job.id });
        const session = runs.find((run) => run.session_id)?.session_id;
        if (session) onOpenSession(session);
        else setSelectedId(job.id);
      } catch (reason) {
        notify("error", String(reason));
      }
    },
    [onOpenSession, notify],
  );

  return (
    <main className="flex min-h-0 flex-1 flex-col">
      <header className="flex min-h-[68px] flex-wrap items-center justify-between gap-3 border-b border-[var(--line)] px-5 sm:px-7">
        <div className="min-w-0">
          <h1 className="text-sm font-semibold">Jobs</h1>
          <p className="text-xs text-[var(--quiet)]">
            {status.paused
              ? "Paused — nothing will start"
              : status.running.length > 0
                ? `${status.running.length} running`
                : "Runs saved instructions on their own"}
          </p>
        </div>
        <div className="flex items-center gap-2">
          {/* One switch that stops every job from starting. A run already going is
              not killed by it: pausing is about what may begin, not what is. */}
          <button type="button" onClick={() => void setPaused(!status.paused)} title={status.paused ? "Resume jobs" : "Pause all jobs"} className="inline-flex min-h-9 items-center gap-2 rounded-md border border-[var(--line)] px-3 text-xs font-medium text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]">
            {status.paused ? <Play size={14} /> : <Pause size={14} />}
            {status.paused ? "Resume" : "Pause all"}
          </button>
          <button type="button" onClick={() => void reload()} aria-label="Refresh jobs" className="grid size-9 place-items-center rounded-md text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]"><RefreshCw size={15} /></button>
          <button type="button" onClick={() => setEditing({ job: null })} className="inline-flex min-h-9 items-center gap-1.5 rounded-md border border-[var(--line)] bg-[var(--panel)] px-3 text-[13px] font-medium text-[var(--text)] hover:bg-[var(--raised)]"><Plus size={15} />New job</button>
        </div>
      </header>

      {editing ? (
        <JobEditor job={editing.job} jobs={jobs} onSubmit={submit} onCancel={() => setEditing(null)} notify={notify} />
      ) : (
        <div className="min-h-0 flex-1 overflow-y-auto p-5 sm:p-7">
          <div className="mx-auto max-w-[860px] space-y-3">
            {loading && jobs.length === 0 && <p className="text-[13px] text-[var(--quiet)]">Loading…</p>}
            {!loading && jobs.length === 0 && (
              <div className="rounded-lg border border-[var(--line)] p-6 text-center">
                <Clock size={20} className="mx-auto mb-2 text-[var(--quiet)]" />
                <p className="text-[13px] text-[var(--muted)]">No jobs yet. Add one and it will run on its own.</p>
              </div>
            )}
            {ordered.map((job, index) => (
              <Fragment key={job.id}>
                {job.lane !== ordered[index - 1]?.lane && (
                  <h3 className="pt-2 text-[11px] font-medium text-[var(--quiet)] first:pt-0">Lane {job.lane}</h3>
                )}
                <JobRow
                  job={job}
                  running={running.has(job.id)}
                  selected={job.id === selectedId}
                  onOpen={() => void openLatest(job)}
                  onHistory={() => setSelectedId(job.id === selectedId ? null : job.id)}
                  onEdit={() => setEditing({ job })}
                  onToggle={(enabled) => void setEnabled(job.id, enabled).catch((reason) => notify("error", String(reason)))}
                  onRunNow={() => void runNow(job.id).then(() => notify("success", "Starting…")).catch((reason) => notify("error", String(reason)))}
                  onDelete={() => void remove(job.id).then(() => notify("success", "Job deleted")).catch((reason) => notify("error", String(reason)))}
                />
              </Fragment>
            ))}
            {selected && <JobHistory job={selected} />}
          </div>
        </div>
      )}
    </main>
  );
}

/**
 * One job in the list.
 *
 * Laid out so a job can be read without clicking: the switch that governs it, what
 * it is, how it runs and on what, and where it stands -- in that order. The name and
 * two lines under it open the last run, because that is what a click is expected to
 * do; the buttons beside them are the things that are *not* "show me".
 */
function JobRow({
  job,
  running,
  selected,
  onOpen,
  onHistory,
  onEdit,
  onToggle,
  onRunNow,
  onDelete,
}: {
  job: Job;
  running: boolean;
  selected: boolean;
  /** Opens the newest run's transcript. */
  onOpen: () => void;
  /** Shows or hides the attempt list. */
  onHistory: () => void;
  onEdit: () => void;
  onToggle: (enabled: boolean) => void;
  onRunNow: () => void;
  onDelete: () => void;
}) {
  return (
    <div className={`rounded-lg border px-3.5 py-2.5 transition-colors ${selected ? "border-[var(--accent)] bg-[color-mix(in_srgb,var(--accent)_8%,transparent)]" : "border-[var(--line)]"}`}>
      <div className="flex items-center gap-3">
        {/* A finished one-shot has nothing left to switch, so the switch gives way
            to a tick: the control that is always in that spot becomes the mark that
            says why there is nothing to control. */}
        {finished(job) ? (
          <span title="Finished" className="grid size-[22px] shrink-0 place-items-center text-[var(--muted)]">
            <Check size={15} />
          </span>
        ) : (
          <Toggle checked={job.enabled} onChange={onToggle} label={`${job.name} switched on`} />
        )}
        <button type="button" onClick={onOpen} title={`${describe(job)} — open the last run`} className="min-w-0 flex-1 text-left">
          <div className="flex items-center gap-2">
            <span className="truncate text-[13.5px] font-medium text-[var(--text)]">{job.name}</span>
            {running && <span className="shrink-0 rounded-full bg-[color-mix(in_srgb,var(--accent)_18%,transparent)] px-2 py-0.5 text-[11px] text-[var(--accent)]">running</span>}
          </div>
          <p className="mt-px truncate text-[12px] text-[var(--muted)]">{howItRuns(job.schedule)} · {modelLabel(job)}</p>
          <p className="mt-px truncate text-[11px] text-[var(--quiet)]">Lane {job.lane} · {whenLine(job)}</p>
        </button>
        <div className="flex shrink-0 items-center gap-0.5">
          <button type="button" onClick={onHistory} aria-expanded={selected} className="rounded px-2 py-1 text-[12px] text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]">{selected ? "Hide runs" : "Runs"}</button>
          <button type="button" onClick={onRunNow} disabled={running} className="rounded px-2 py-1 text-[12px] text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)] disabled:opacity-40">Run now</button>
          <button type="button" onClick={onEdit} className="rounded px-2 py-1 text-[12px] text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]">Edit</button>
          <button type="button" onClick={onDelete} aria-label={`Delete ${job.name}`} className="grid size-7 place-items-center rounded text-[var(--quiet)] hover:bg-[var(--raised)] hover:text-[var(--danger)]"><Trash2 size={13} /></button>
        </div>
      </div>
    </div>
  );
}

/** One job's recent attempts. */
function JobHistory({ job }: { job: Job }) {
  const { runs } = useJobRuns(job.id);
  return (
    <div className="mt-4 rounded-lg border border-[var(--line)] p-4">
      <h2 className="mb-2 text-[13px] font-medium text-[var(--text)]">History — {job.name}</h2>
      {runs.length === 0 && <p className="text-[12px] text-[var(--quiet)]">No attempts yet.</p>}
      <ul className="space-y-1">
        {runs.map((run) => (
          <li key={run.id} className="flex items-baseline gap-2 text-[12px]">
            <span className={`shrink-0 font-mono ${run.status === "completed" ? "text-[var(--added)]" : run.status === "failed" ? "text-[var(--danger)]" : "text-[var(--muted)]"}`}>{run.status}</span>
            <span className="shrink-0 text-[var(--quiet)]">{shortTime(run.started_at)}</span>
            {run.reason && <span className="min-w-0 truncate text-[var(--muted)]" title={run.reason}>{run.reason}</span>}
          </li>
        ))}
      </ul>
    </div>
  );
}

/** A one-line summary of everything about a job, for the row's tooltip. */
function describe(job: Job): string {
  const limits = [
    job.conditions.max_ram_percent != null ? `RAM<${job.conditions.max_ram_percent}%` : null,
    job.conditions.max_vram_percent != null ? `VRAM<${job.conditions.max_vram_percent}%` : null,
    job.conditions.min_idle_seconds ? `idle>${job.conditions.min_idle_seconds}s` : null,
  ].filter(Boolean).join(", ");
  return `${howItRuns(job.schedule)} · ${modelLabel(job)}${limits ? ` · ${limits}` : ""}${job.conditions.stop_when_busy ? " · yields when busy" : ""}`;
}

/** A one-shot that has had its run: nothing is scheduled, so the job is over. */
function finished(job: Job): boolean {
  return job.schedule.kind === "once" && job.last_status !== null && job.next_run_at === null;
}

/** Where a job sits inside its lane: running, then due, then off, then over. */
function rank(job: Job, running: boolean): number {
  if (running) return 0;
  if (finished(job)) return 3;
  return job.enabled ? 1 : 2;
}

/** How the job runs, in the few words the card has room for. */
function howItRuns(schedule: JobSchedule): string {
  switch (schedule.kind) {
    case "once":
      return schedule.at ? `Once, ${shortTime(schedule.at)}` : "Once, as soon as possible";
    case "every":
      return `Every ${schedule.minutes} min`;
    case "daily":
      return `Every day at ${schedule.at}`;
    case "weekly":
      return `Every ${WEEKDAYS[schedule.weekday] ?? "week"}${schedule.at ? ` at ${schedule.at}` : ""}`;
  }
}

/** What the job runs on. */
function modelLabel(job: Job): string {
  return job.model.kind === "local" ? "Local model" : job.model.model;
}

/** When it last finished, and when it is next due. */
function whenLine(job: Job): string {
  const last = job.last_status
    ? `${job.last_status === "completed" ? "Completed" : "Ended"}${job.last_run_at ? ` ${shortTime(job.last_run_at)}` : ""}`
    : "Not run yet";
  if (!job.enabled) return `${last} · switched off`;
  return `${last} · next ${job.next_run_at ? shortTime(job.next_run_at) : "unset"}`;
}

const WEEKDAYS = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

/** A stored UTC timestamp as the reader's local time, to the minute. */
function shortTime(stamp: string): string {
  const at = new Date(stamp);
  return Number.isNaN(at.getTime()) ? stamp : at.toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
}
