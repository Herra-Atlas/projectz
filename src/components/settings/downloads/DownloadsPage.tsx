import { Pause, Play, X } from "lucide-react";

import { formatBytes, formatSpeed } from "../../../features/downloads/format";
import type { DownloadJob } from "../../../features/downloads/types";
import type { Notify } from "../../../features/notifications/types";
import { useDownloads } from "../../../features/downloads/useDownloads";
import { SettingsPageTitle } from "../../SettingsSection";

/**
 * Everything the app is downloading, and everything it has downloaded since it
 * started.
 *
 * Session-scoped on purpose: this is work, not a library. The files are the
 * record, and a list that survived a restart would be a second, staler index of
 * the same thing.
 *
 * The one loud element is the progress bar. A download page whose every row is a
 * card with a border and a badge reads as a dashboard; a numbered line with one
 * accent fill reads as a transfer, which is what it is.
 */
export function DownloadsPage({ notify }: { notify: Notify }) {
  const { jobs, pause, resume, stop } = useDownloads((message) => notify("error", message));

  const active = jobs.filter((job) => job.state === "running" || job.state === "paused");
  const finished = jobs.filter((job) => job.state !== "running" && job.state !== "paused");

  return (
    <>
      <SettingsPageTitle>
        <h2 className="text-[17px] font-semibold tracking-tight">Downloads</h2>
      </SettingsPageTitle>

      {jobs.length === 0 ? (
        <div className="px-4 py-8">
          <p className="text-sm text-[var(--muted)]">Nothing has been downloaded this session.</p>
          <p className="mt-1 text-xs text-[var(--quiet)]">
            Models and voices download from the engine they belong to, under Local.
          </p>
        </div>
      ) : (
        <div className="space-y-7">
          {active.length > 0 && (
            <section>
              <h3 className="mb-1.5 px-1 text-[11px] font-medium text-[var(--quiet)]">
                In progress · {active.length}
              </h3>
              <ul className="divide-y divide-[var(--line)] border-y border-[var(--line)]">
                {active.map((job) => (
                  <ActiveRow
                    key={job.id}
                    job={job}
                    onPause={() => pause(job.id)}
                    onResume={() => resume(job.id)}
                    onStop={() => stop(job.id)}
                  />
                ))}
              </ul>
            </section>
          )}

          {finished.length > 0 && (
            <section>
              <h3 className="mb-1.5 px-1 text-[11px] font-medium text-[var(--quiet)]">
                Finished this session
              </h3>
              <ul className="divide-y divide-[var(--line)] border-y border-[var(--line)]">
                {finished.map((job) => (
                  <FinishedRow key={job.id} job={job} />
                ))}
              </ul>
            </section>
          )}
        </div>
      )}
    </>
  );
}

/** A transfer that is still moving, or waiting to. */
function ActiveRow({
  job,
  onPause,
  onResume,
  onStop,
}: {
  job: DownloadJob;
  onPause: () => void;
  onResume: () => void;
  onStop: () => void;
}) {
  const percent =
    job.total && job.total > 0 ? Math.min(100, (job.received / job.total) * 100) : null;
  const paused = job.state === "paused";
  const remaining = timeRemaining(job);

  return (
    <li className="px-4 py-3">
      <div className="flex items-baseline gap-3">
        <div className="min-w-0 flex-1">
          <h4 className="truncate text-[13px] font-medium text-[var(--text)]" title={job.path}>
            {job.name}
          </h4>
          <p className="truncate text-[11px] text-[var(--quiet)]">
            {job.repo}
            {job.total
              ? ` · ${formatBytes(job.received)} of ${formatBytes(job.total)}`
              : ` · ${formatBytes(job.received)}`}
            {!paused && remaining !== null ? ` · ${remaining} left` : ""}
          </p>
        </div>
        <span className="shrink-0 text-[13px] tabular-nums text-[var(--text)]">
          {percent === null ? "—" : `${percent.toFixed(0)}%`}
        </span>
        <span className="hidden w-20 shrink-0 text-right text-[11px] tabular-nums text-[var(--muted)] sm:block">
          {paused ? "paused" : formatSpeed(job.speed)}
        </span>
        <div className="flex shrink-0 items-center gap-0.5">
          {paused ? (
            <button
              type="button"
              onClick={onResume}
              aria-label={`Resume ${job.name}`}
              title="Resume"
              className="grid size-8 place-items-center rounded-md text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
            >
              <Play size={14} />
            </button>
          ) : (
            <button
              type="button"
              onClick={onPause}
              aria-label={`Pause ${job.name}`}
              title="Pause"
              className="grid size-8 place-items-center rounded-md text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
            >
              <Pause size={14} />
            </button>
          )}
          <button
            type="button"
            onClick={onStop}
            aria-label={`Stop ${job.name}`}
            title="Stop and discard the partial file"
            className="grid size-8 place-items-center rounded-md text-[var(--quiet)] hover:bg-[var(--raised)] hover:text-[var(--danger)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
          >
            <X size={14} />
          </button>
        </div>
      </div>

      {/* The bar is the row's baseline: it belongs to this transfer and to nothing
          else on the page, so it is drawn here rather than as a border. */}
      <div className="mt-2 h-[3px] overflow-hidden rounded-full bg-[var(--line)]">
        <div
          className="h-full rounded-full bg-[var(--accent)] transition-[width] duration-300"
          style={{ width: `${percent ?? 0}%` }}
        />
      </div>
    </li>
  );
}

/**
 * Time left at the rate the transfer is actually running at.
 *
 * The same smoothed speed the row prints, so the two figures agree. Nothing is
 * shown until the first sample lands and while the rate is zero -- a division by
 * zero would be "infinite", and a guess from one packet would jump around.
 */
function timeRemaining(job: DownloadJob): string | null {
  if (!job.total || job.speed <= 0) return null;
  const seconds = Math.max(0, (job.total - job.received) / job.speed);
  if (seconds < 60) return `${Math.max(1, Math.round(seconds))}s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ${Math.round(seconds % 60)}s`;
  return `${Math.floor(seconds / 3600)}h ${Math.round((seconds % 3600) / 60)}m`;
}

/** A transfer that is over, however it ended. */
function FinishedRow({ job }: { job: DownloadJob }) {  const failed = job.state === "failed";
  const stopped = job.state === "stopped";
  const label = failed ? "Failed" : stopped ? "Stopped" : "Installed";

  return (
    <li className="flex items-baseline gap-3 px-4 py-2.5">
      <div className="min-w-0 flex-1">
        <h4 className="truncate text-[13px] text-[var(--text)]" title={job.path}>
          {job.name}
        </h4>
        <p className="truncate text-[11px] text-[var(--quiet)]">
          {failed || stopped ? job.error ?? job.repo : job.repo}
        </p>
      </div>
      <span className="hidden shrink-0 text-[11px] tabular-nums text-[var(--muted)] sm:block">
        {formatBytes(job.received)}
      </span>
      <span
        className={`w-20 shrink-0 text-right text-[11px] ${failed ? "text-[var(--danger)]" : "text-[var(--quiet)]"}`}
      >
        {label}
      </span>
    </li>
  );
}
