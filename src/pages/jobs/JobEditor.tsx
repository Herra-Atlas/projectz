import { useState } from "react";
import { CircleHelp } from "lucide-react";
import { Toggle } from "../../components/SettingsSection";
import { useModelRegistry } from "../../features/models/useModelRegistry";
import { useMachineTotals } from "../../features/jobs/useJobs";
import { useWorkspaces, workspaceLabel } from "../../features/workspace/useWorkspaces";
import type { Notify } from "../../features/notifications/types";
import { ALL_ACCESS, DEFAULT_CONDITIONS, draftJob, toDraft, type Job, type JobSchedule, type NewJob } from "../../features/jobs/types";

/**
 * The job editor.
 *
 * One form for create and edit, because they are the same form with a different
 * starting value -- two would be the same fields maintained twice and would drift
 * the first time a condition was added.
 *
 * The draft is local state, copied from the job when the editor opens. Saving sends
 * the whole draft rather than a patch: the editor is the only writer, and a whole
 * record is one thing to validate instead of a set of fields that only make sense
 * together.
 */
export default function JobEditor({
  job,
  jobs,
  onSubmit,
  onCancel,
  notify,
}: {
  /** The job being edited, or `null` for a new one. */
  job: Job | null;
  /** Every other job, for the dependency list. */
  jobs: Job[];
  onSubmit: (draft: NewJob) => Promise<void>;
  onCancel: () => void;
  notify: Notify;
}) {
  const { endpoints, localModels } = useModelRegistry(0);
  const totals = useMachineTotals();
  const { workspaces } = useWorkspaces(true);
  const [draft, setDraft] = useState<NewJob>(() => (job ? toDraft(job) : draftJob()));
  const [saving, setSaving] = useState(false);

  // The schedule is one choice from two families -- once, or repeatedly -- and the
  // editor keeps the sub-choice and the values for each. "Once, at 15:00 today" and
  // "once, as soon as possible" are the same stored shape (an absolute moment, or
  // none), so the moment is worked out here and the backend only ever reads one
  // form.
  const stored = job?.schedule;
  const [mode, setMode] = useState<"once" | "repeat">(() =>
    stored && stored.kind !== "once" ? "repeat" : "once",
  );
  const [onceKind, setOnceKind] = useState<"asap" | "today" | "date">(() =>
    stored?.kind === "once" && stored.at
      ? isToday(new Date(stored.at)) ? "today" : "date"
      : "asap",
  );
  const [onceTodayTime, setOnceTodayTime] = useState(() =>
    stored?.kind === "once" && stored.at ? timeInput(new Date(stored.at)) : "09:00",
  );
  const [onceDate, setOnceDate] = useState(() =>
    stored?.kind === "once" && stored.at ? dateInput(new Date(stored.at)) : dateInput(new Date()),
  );
  const [onceDateTime, setOnceDateTime] = useState("09:00");

  const [repeatKind, setRepeatKind] = useState<"every" | "daily" | "weekly">(() =>
    stored?.kind === "every" ? "every" : stored?.kind === "weekly" ? "weekly" : "daily",
  );
  const [everyMinutes, setEveryMinutes] = useState(() => (stored?.kind === "every" ? stored.minutes : 60));
  const [dailyAt, setDailyAt] = useState(() => (stored?.kind === "daily" ? stored.at : "03:00"));
  const [weekday, setWeekday] = useState(() => (stored?.kind === "weekly" ? stored.weekday : 0));
  const [weeklyAt, setWeeklyAt] = useState(() => (stored?.kind === "weekly" ? stored.at ?? "" : ""));

  const composeSchedule = (): JobSchedule => {
    if (mode === "once") {
      if (onceKind === "asap") return { kind: "once", at: null };
      if (onceKind === "today") return { kind: "once", at: localMoment(dateInput(new Date()), onceTodayTime) };
      return { kind: "once", at: localMoment(onceDate, onceDateTime) };
    }
    if (repeatKind === "every") return { kind: "every", minutes: Math.max(1, everyMinutes) };
    if (repeatKind === "daily") return { kind: "daily", at: dailyAt };
    // An empty time means "as soon as possible that day".
    return { kind: "weekly", weekday, at: weeklyAt || null };
  };

  const patch = (changes: Partial<NewJob>) => setDraft((current) => ({ ...current, ...changes }));
  // Held as a value rather than narrowed at each use: the narrowing a union gives
  // does not survive into the callbacks below, which run after the check.
  const remote = draft.model.kind === "remote" ? draft.model : null;
  const endpoint = remote ? endpoints.find((item) => item.id === remote.endpoint_id) : undefined;

  const save = async () => {
    setSaving(true);
    try {
      await onSubmit({ ...draft, schedule: composeSchedule() });
    } catch (reason) {
      notify("error", String(reason));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="min-h-0 flex-1 overflow-y-auto p-5 sm:p-7">
      <div className="mx-auto max-w-[760px] space-y-6">
        <header className="flex items-center justify-between gap-3 border-b border-[var(--line)] pb-4">
          <h2 className="text-[15px] font-semibold">{job ? "Edit job" : "New job"}</h2>
          <div className="flex items-center gap-2">
            <button type="button" onClick={onCancel} className="min-h-9 rounded-lg border border-[var(--line)] px-3 text-[13px] text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]">Cancel</button>
            <button type="button" onClick={() => void save()} disabled={saving || !draft.name.trim() || !draft.prompt.trim()} className="min-h-9 rounded-lg bg-[var(--accent)] px-3 text-[13px] font-medium text-[var(--accent-ink)] disabled:opacity-40">{saving ? "Saving…" : "Save"}</button>
          </div>
        </header>

        <Field label="Name" hint="What the job is called in the list, and the title its transcript is saved under.">
          <input value={draft.name} onChange={(event) => patch({ name: event.target.value })} placeholder="Nightly summary" className="min-h-9 w-full rounded-lg border border-[var(--line)] bg-[var(--page)] px-3 text-[13px] text-[var(--text)] outline-none focus-visible:border-[var(--accent)]" />
        </Field>

        <Field label="Instruction" hint="What the job should do, in your own words. It runs as one message.">
          <textarea value={draft.prompt} onChange={(event) => patch({ prompt: event.target.value })} rows={6} placeholder="Summarise what changed in the workspace today and write it to notes.md" className="w-full rounded-lg border border-[var(--line)] bg-[var(--page)] px-3 py-2 text-[13px] text-[var(--text)] outline-none focus-visible:border-[var(--accent)]" />
        </Field>

        <Field label="Model" hint="Which model runs the instruction. A local model is started for the run if it is not already loaded.">
          <div className="space-y-2">
            <select value={draft.model.kind} onChange={(event) => {
              const kind = event.target.value as "local" | "remote";
              // A job's "stop when busy" default follows the model: a local run
              // costs the PC, a remote one does not.
              patch({
                model: kind === "local"
                  ? { kind: "local", id: localModels[0]?.id ?? "" }
                  : { kind: "remote", endpoint_id: endpoints[0]?.id ?? "", model: endpoints[0]?.models[0] ?? "" },
                conditions: { ...draft.conditions, stop_when_busy: kind === "local" },
              });
            }} className="min-h-9 rounded-lg border border-[var(--line)] bg-[var(--page)] px-2 text-[13px] text-[var(--text)]">
              <option value="local">Local model</option>
              <option value="remote">Provider model</option>
            </select>
            {draft.model.kind === "local" ? (
              <select value={draft.model.id} onChange={(event) => patch({ model: { kind: "local", id: event.target.value } })} className="min-h-9 w-full rounded-lg border border-[var(--line)] bg-[var(--page)] px-2 text-[13px] text-[var(--text)]">
                {localModels.length === 0 && <option value="">No local models added</option>}
                {localModels.map((model) => <option key={model.id} value={model.id}>{model.name}</option>)}
              </select>
            ) : (
              <div className="flex gap-2">
                <select value={draft.model.endpoint_id} onChange={(event) => {
                  const next = endpoints.find((item) => item.id === event.target.value);
                  patch({ model: { kind: "remote", endpoint_id: event.target.value, model: next?.models[0] ?? "" } });
                }} className="min-h-9 flex-1 rounded-lg border border-[var(--line)] bg-[var(--page)] px-2 text-[13px] text-[var(--text)]">
                  {endpoints.length === 0 && <option value="">No providers configured</option>}
                  {endpoints.map((item) => <option key={item.id} value={item.id}>{item.name}</option>)}
                </select>
                <select value={draft.model.model} onChange={(event) => patch({ model: { kind: "remote", endpoint_id: remote?.endpoint_id ?? "", model: event.target.value } })} className="min-h-9 flex-1 rounded-lg border border-[var(--line)] bg-[var(--page)] px-2 text-[13px] text-[var(--text)]">
                  {(endpoint?.models ?? []).map((name) => <option key={name} value={name}>{name}</option>)}
                </select>
              </div>
            )}
          </div>
        </Field>

        <Field label="Schedule" hint="Once runs a single time and then stops. Repeatedly keeps running until you switch the job off.">
          <div className="space-y-3 text-[13px] text-[var(--text)]">
            <div className="flex gap-4">
              {(["once", "repeat"] as const).map((option) => (
                <label key={option} className="flex items-center gap-2">
                  <input type="radio" checked={mode === option} onChange={() => setMode(option)} />
                  {option === "once" ? "Once" : "Repeatedly"}
                </label>
              ))}
            </div>

            {mode === "once" ? (
              <div className="space-y-2 border-l border-[var(--line)] pl-3">
                <label className="flex items-center gap-2">
                  <input type="radio" checked={onceKind === "asap"} onChange={() => setOnceKind("asap")} />
                  As soon as possible
                </label>
                <label className="flex items-center gap-2">
                  <input type="radio" checked={onceKind === "today"} onChange={() => setOnceKind("today")} />
                  Today at
                  <input type="time" value={onceTodayTime} disabled={onceKind !== "today"} onChange={(event) => setOnceTodayTime(event.target.value)} className="min-h-9 rounded-lg border border-[var(--line)] bg-[var(--page)] px-2 disabled:opacity-40" />
                </label>
                <label className="flex flex-wrap items-center gap-2">
                  <input type="radio" checked={onceKind === "date"} onChange={() => setOnceKind("date")} />
                  On
                  <input type="date" value={onceDate} disabled={onceKind !== "date"} onChange={(event) => setOnceDate(event.target.value)} className="min-h-9 rounded-lg border border-[var(--line)] bg-[var(--page)] px-2 disabled:opacity-40" />
                  at
                  <input type="time" value={onceDateTime} disabled={onceKind !== "date"} onChange={(event) => setOnceDateTime(event.target.value)} className="min-h-9 rounded-lg border border-[var(--line)] bg-[var(--page)] px-2 disabled:opacity-40" />
                </label>
                <p className="text-[12px] text-[var(--quiet)]">Runs once at that moment, then never again.</p>
              </div>
            ) : (
              <div className="space-y-2 border-l border-[var(--line)] pl-3">
                <label className="flex items-center gap-2">
                  <input type="radio" checked={repeatKind === "every"} onChange={() => setRepeatKind("every")} />
                  Every
                  <input type="number" min={1} value={everyMinutes} disabled={repeatKind !== "every"} onChange={(event) => setEveryMinutes(Number(event.target.value))} className="min-h-9 w-20 rounded-lg border border-[var(--line)] bg-[var(--page)] px-2 disabled:opacity-40" />
                  minutes
                </label>
                <label className="flex items-center gap-2">
                  <input type="radio" checked={repeatKind === "daily"} onChange={() => setRepeatKind("daily")} />
                  Every day at
                  <input type="time" value={dailyAt} disabled={repeatKind !== "daily"} onChange={(event) => setDailyAt(event.target.value)} className="min-h-9 rounded-lg border border-[var(--line)] bg-[var(--page)] px-2 disabled:opacity-40" />
                </label>
                <label className="flex flex-wrap items-center gap-2">
                  <input type="radio" checked={repeatKind === "weekly"} onChange={() => setRepeatKind("weekly")} />
                  On
                  <select value={weekday} disabled={repeatKind !== "weekly"} onChange={(event) => setWeekday(Number(event.target.value))} className="min-h-9 rounded-lg border border-[var(--line)] bg-[var(--page)] px-2 disabled:opacity-40">
                    {WEEKDAYS.map((day, index) => <option key={day} value={index}>{day}</option>)}
                  </select>
                  at
                  <input type="time" value={weeklyAt} disabled={repeatKind !== "weekly"} onChange={(event) => setWeeklyAt(event.target.value)} className="min-h-9 rounded-lg border border-[var(--line)] bg-[var(--page)] px-2 disabled:opacity-40" />
                  <span className="text-[12px] text-[var(--quiet)]">empty = as soon as that day arrives</span>
                </label>
              </div>
            )}
          </div>
        </Field>

        <Field label="Machine" hint="The job waits for these before it starts. Once running it is judged only after a two-minute warm-up — loading a model raises memory by itself — and only if a limit stays exceeded for 20 seconds. Switch off `Stop if the PC gets busy` to never stop on resources, however long the job takes.">
          <div className="grid gap-4 sm:grid-cols-2">
            <PercentField label="Max RAM" value={draft.conditions.max_ram_percent} total={totals.ram_total_bytes} onChange={(value) => patch({ conditions: { ...draft.conditions, max_ram_percent: value } })} />
            {totals.vram_total_bytes !== null ? (
              <PercentField label="Max VRAM" value={draft.conditions.max_vram_percent} total={totals.vram_total_bytes} onChange={(value) => patch({ conditions: { ...draft.conditions, max_vram_percent: value } })} />
            ) : (
              // Offered only when the figure can actually be read. A slider that
              // cannot be evaluated is a control that lies, which is worse than
              // saying why it is missing.
              <div className="text-[12px] text-[var(--muted)]">
                <div className="mb-1 flex items-center justify-between gap-2">
                  <span>Max VRAM</span>
                  {draft.conditions.max_vram_percent !== null && (
                    <button type="button" onClick={() => patch({ conditions: { ...draft.conditions, max_vram_percent: null } })} className="text-[11px] text-[var(--quiet)] hover:text-[var(--text)]">Clear {draft.conditions.max_vram_percent}%</button>
                  )}
                </div>
                <p className="text-[var(--quiet)]">Unavailable: this build cannot read the GPU's memory, so a VRAM limit could never be checked. A stored one is ignored.</p>
              </div>
            )}
            <NumberField label="Idle for (seconds)" value={draft.conditions.min_idle_seconds} onChange={(value) => patch({ conditions: { ...draft.conditions, min_idle_seconds: value } })} />
            <div className="flex items-center justify-between gap-3 self-end text-[13px] text-[var(--text)]">
              <span>Stop if the PC gets busy</span>
              <Toggle checked={draft.conditions.stop_when_busy} onChange={(value) => patch({ conditions: { ...draft.conditions, stop_when_busy: value } })} label="Stop if the PC gets busy" />
            </div>
          </div>
        </Field>

        <Field label="Workspace" hint="The folder the job works in. The tools are pointed at it for the run, then put back.">
          <select value={draft.workspace ?? ""} onChange={(event) => patch({ workspace: event.target.value || null })} className="min-h-9 w-full rounded-lg border border-[var(--line)] bg-[var(--page)] px-2 text-[13px] text-[var(--text)]">
            <option value="">The workspace that is open</option>
            {workspaces.paths.map((path) => (
              <option key={path} value={path}>{workspaceLabel(path)} — {path}</option>
            ))}
          </select>
        </Field>

        <Field label="Access" hint="What this job's agent may use. A tool that is off is not offered to the model at all.">
          <div className="flex flex-wrap gap-x-5 gap-y-3 text-[13px]">
            {(["read", "write", "terminal", "web", "subagents"] as const).map((capability) => (
              <label key={capability} className="flex items-center gap-2 capitalize text-[var(--text)]">
                <Toggle checked={draft.access[capability]} onChange={(value) => patch({ access: { ...draft.access, [capability]: value } })} label={capability} />
                {capability}
              </label>
            ))}
          </div>
        </Field>

        <Field label="Lane" hint="Jobs in one lane never run at once: a job waits for the earlier one in its lane to finish before it starts. Give two jobs different lanes to let them overlap.">
          <input value={draft.lane} onChange={(event) => patch({ lane: event.target.value })} className="min-h-9 w-full rounded-lg border border-[var(--line)] bg-[var(--page)] px-3 text-[13px] text-[var(--text)]" />
        </Field>

        {jobs.filter((other) => other.id !== job?.id).length > 0 && (
          <Field label="Wait for" hint="This job will not start until each of these has finished successfully at least once.">
            <div className="flex flex-wrap gap-x-5 gap-y-3 text-[13px]">
              {jobs.filter((other) => other.id !== job?.id).map((other) => (
                <label key={other.id} className="flex items-center gap-2 text-[var(--text)]">
                  <Toggle checked={draft.depends_on.includes(other.id)} onChange={(value) => patch({ depends_on: value ? [...draft.depends_on, other.id] : draft.depends_on.filter((id) => id !== other.id) })} label={`Wait for ${other.name}`} />
                  {other.name}
                </label>
              ))}
            </div>
          </Field>
        )}

        <div className="flex items-center justify-between gap-3 text-[13px] text-[var(--text)]">
          <span>Enabled</span>
          <Toggle checked={draft.enabled} onChange={(value) => patch({ enabled: value })} label="Enabled" />
        </div>

        {/* Access is the narrowing control, so the four presets are shown as what
            they mean rather than as a level to decode. */}
        <div className="flex flex-wrap items-center gap-2 border-t border-[var(--line)] pt-4 text-[12px] text-[var(--muted)]">
          <span>Presets:</span>
          {[
            // Written as what each preset *changes*, so a capability added later is on
            // in all three unless a preset says otherwise.
            { label: "Read only", access: { ...ALL_ACCESS, write: false, terminal: false, subagents: false } },
            { label: "Read + write", access: { ...ALL_ACCESS, terminal: false, subagents: false } },
            { label: "Everything", access: { ...ALL_ACCESS } },
          ].map((preset) => (
            <button key={preset.label} type="button" onClick={() => patch({ access: preset.access })} className="rounded-md border border-[var(--line)] px-2 py-1 hover:bg-[var(--raised)] hover:text-[var(--text)]">{preset.label}</button>
          ))}
          <button type="button" onClick={() => patch({ conditions: { ...DEFAULT_CONDITIONS } })} className="rounded-md border border-[var(--line)] px-2 py-1 hover:bg-[var(--raised)] hover:text-[var(--text)]">Reset machine limits</button>
        </div>
      </div>
    </div>
  );
}

/**
 * One section of the editor.
 *
 * The explanation is a `?` beside the label rather than a paragraph under it: with
 * a dozen sections, a paragraph each buried the fields they were describing.
 * Hovering the mark answers "what does this do" without the answer taking up room
 * when nobody asked.
 */
function Field({ label, hint, children }: { label: string; hint?: string; children: React.ReactNode }) {
  return (
    <div>
      <div className="mb-1.5 flex items-center gap-1.5 text-[13px] font-medium text-[var(--text)]">
        <span>{label}</span>
        {hint && (
          <span title={hint} className="cursor-help text-[var(--quiet)] transition-colors hover:text-[var(--text)]">
            <CircleHelp size={13} aria-label={hint} />
          </span>
        )}
      </div>
      {children}
    </div>
  );
}

function NumberField({ label, value, onChange }: { label: string; value: number | null; onChange: (value: number | null) => void }) {
  return (
    <label className="text-[12px] text-[var(--muted)]">
      <span className="mb-1 block">{label}</span>
      <input
        type="number"
        // Empty means "no limit", which is distinct from zero: zero would block
        // every run, and clearing the field is how a user says they do not care.
        value={value ?? ""}
        onChange={(event) => onChange(event.target.value === "" ? null : Number(event.target.value))}
        className="min-h-9 w-full rounded-lg border border-[var(--line)] bg-[var(--page)] px-2 text-[13px] text-[var(--text)]"
      />
    </label>
  );
}

/**
 * A limit chosen as a percentage, with the bytes it means beside it.
 *
 * A slider rather than a box, like the context and parallel-slot controls in
 * Settings, because the number is never what the user is thinking in: "do not start
 * while the PC is 60% full" is the decision, and "≈5.8 GB" is only how you check
 * it. The total comes from the backend, so the figure is this machine's real
 * memory and not a number typed in twice.
 *
 * "No limit" is a state of its own rather than 0%: zero would block every run, and
 * turning the limit off is what an empty value means everywhere else here.
 */
function PercentField({
  label,
  value,
  total,
  onChange,
}: {
  label: string;
  value: number | null;
  total: number | null;
  onChange: (value: number | null) => void;
}) {
  return (
    <div className="text-[12px] text-[var(--muted)]">
      <div className="mb-1 flex items-center justify-between gap-2">
        <span>{label}</span>
        <span
          className="tabular-nums text-[var(--text)]"
          title={total ? undefined : "This build cannot read the total, so only the percentage is enforced"}
        >
          {value === null ? "no limit" : total ? `${value}% ≈ ${gigabytes(total, value)}` : `${value}%`}
        </span>
      </div>
      <div className="flex items-center gap-2">
        <input
          type="range"
          min={0}
          max={100}
          step={1}
          // Parked at the right while the limit is off, so dragging left is what
          // switches it on -- the track is the control, as in Settings.
          value={value ?? 100}
          onChange={(event) => onChange(Number(event.target.value))}
          aria-label={`${label} limit`}
          className="h-1 min-w-0 flex-1 cursor-pointer appearance-none rounded-full bg-[var(--line)] accent-[var(--accent)]"
        />
        <button type="button" onClick={() => onChange(null)} className="shrink-0 text-[11px] text-[var(--quiet)] hover:text-[var(--text)]">Off</button>
      </div>
    </div>
  );
}

/** The bytes a percentage of a total is. */
function gigabytes(total: number, percent: number): string {
  return `${((total * percent) / 100 / 1024 ** 3).toFixed(1)} GB`;
}

const WEEKDAYS = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

/** `YYYY-MM-DD` in local time, the value a date field wants. */
function dateInput(date: Date): string {
  const pad = (part: number) => String(part).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/** `HH:MM` in local time. */
function timeInput(date: Date): string {
  const pad = (part: number) => String(part).padStart(2, "0");
  return `${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

/** Whether a moment falls on today, in local time. */
function isToday(date: Date): boolean {
  return dateInput(date) === dateInput(new Date());
}

/**
 * A local date and time as the UTC moment the backend stores.
 *
 * `new Date("2026-01-02T03:00")` with no zone is read as local time, which is what
 * the fields mean: the user picked a wall-clock moment where they are.
 */
function localMoment(date: string, time: string): string | null {
  if (!date) return null;
  const at = new Date(`${date}T${time || "00:00"}`);
  return Number.isNaN(at.getTime()) ? null : at.toISOString();
}
