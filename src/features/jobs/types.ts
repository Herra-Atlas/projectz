/**
 * A scheduled job, as the backend stores and returns it.
 *
 * The shapes here mirror `src-tauri/src/database/jobs.rs` field for field. They
 * are snake_case because that is what Rust serializes: unlike the chat types,
 * this record crosses the boundary unchanged rather than being mapped, so the
 * fewer places a name can be mistranslated the better.
 */

/** What a job is allowed to touch. Mirrors the Rust `AccessSet`. */
export type AccessSet = {
  read: boolean;
  write: boolean;
  terminal: boolean;
  web: boolean;
  subagents: boolean;
  /**
   * Whether the run may schedule work for later.
   *
   * Not offered as a switch anywhere: a job's own run always has it taken away, so
   * scheduled work cannot schedule more of itself. It is here because the set has to
   * round-trip whole.
   */
  jobs: boolean;
};

/** Everything on, which is what a hand-started run gets. */
export const ALL_ACCESS: AccessSet = {
  read: true,
  write: true,
  terminal: true,
  web: true,
  subagents: true,
  jobs: true,
};

/** Which model a job runs on. A local model is named by id; a remote pair is not. */
export type JobModel =
  | { kind: "local"; id: string }
  | { kind: "remote"; endpoint_id: string; model: string };

/**
 * When a job is due.
 *
 * One shape at a time, matching the Rust enum: the editor offers them as one
 * choice, and two at once would be a state that means nothing. `weekday` is
 * `0..=6`, Monday first.
 */
export type JobSchedule =
  | { kind: "once"; /** A UTC moment, or `null` for "as soon as possible". */ at: string | null }
  | { kind: "every"; minutes: number }
  | { kind: "daily"; /** `"HH:MM"` local. */ at: string }
  | { kind: "weekly"; weekday: number; /** `"HH:MM"` local, or `null` for "that day". */ at: string | null };

/** What the machine must look like before a job starts, and whether it may be cut
 *  short once it has. */
export type JobConditions = {
  max_ram_percent: number | null;
  max_vram_percent: number | null;
  min_idle_seconds: number | null;
  /** Off means the run finishes whatever else the PC is doing. */
  stop_when_busy: boolean;
};

/**
 * The default conditions for a new job.
 *
 * `stop_when_busy` is on here, which is the answer for a local model: the run
 * costs the machine something, so it should yield. The editor turns it off when a
 * remote model is chosen, because then it costs the PC nothing.
 */
export const DEFAULT_CONDITIONS: JobConditions = {
  max_ram_percent: 60,
  max_vram_percent: 30,
  min_idle_seconds: null,
  stop_when_busy: true,
};

export type Job = {
  id: string;
  name: string;
  prompt: string;
  workspace: string | null;
  mode: "chat" | "agent";
  permission: "ask" | "auto_safe" | "auto_writes" | "full";
  access: AccessSet;
  model: JobModel;
  schedule: JobSchedule;
  conditions: JobConditions;
  lane: string;
  depends_on: string[];
  enabled: boolean;
  next_run_at: string | null;
  last_run_at: string | null;
  last_status: string | null;
  created_at: string;
  updated_at: string;
};

/** What the editor sends. Everything the scheduler owns is left out. */
export type NewJob = {
  name: string;
  prompt: string;
  workspace: string | null;
  mode: "chat" | "agent";
  permission: Job["permission"];
  access: AccessSet;
  model: JobModel;
  schedule: JobSchedule;
  conditions: JobConditions;
  lane: string;
  depends_on: string[];
  enabled: boolean;
};

/** One attempt at a job. */
export type JobRun = {
  id: string;
  job_id: string;
  /** The session the attempt ran as, so its transcript can be opened. */
  session_id: string | null;
  status: string;
  reason: string | null;
  started_at: string;
  finished_at: string | null;
  created_at: string;
};

/** What the scheduler broadcasts while it works. */
export type JobEvent = {
  jobId: string;
  name: string;
  status: string;
  reason?: string | null;
};

/** The scheduler's own state, for the pause switch and the running list. */
export type SchedulerStatus = {
  paused: boolean;
  running: string[];
};

/** A new job with the defaults the editor opens on. */
export function draftJob(): NewJob {
  return {
    name: "",
    prompt: "",
    workspace: null,
    mode: "agent",
    // Full is the honest default for an unattended run: nobody is there to answer
    // a prompt, so asking would park the job forever. The access set is what
    // narrows it, not the level.
    permission: "full",
    access: { ...ALL_ACCESS },
    model: { kind: "local", id: "" },
    schedule: { kind: "once", at: null },
    conditions: { ...DEFAULT_CONDITIONS },
    lane: "default",
    depends_on: [],
    enabled: true,
  };
}

/** A stored job as an editable draft. */
export function toDraft(job: Job): NewJob {
  return {
    name: job.name,
    prompt: job.prompt,
    workspace: job.workspace,
    mode: job.mode,
    permission: job.permission,
    access: { ...job.access },
    model: { ...job.model },
    schedule: job.schedule,
    conditions: { ...job.conditions },
    lane: job.lane,
    depends_on: [...job.depends_on],
    enabled: job.enabled,
  };
}
