/** The kinds of file the downloader installs. Mirrors `downloads::Kind`. */
export type DownloadKind = "llama" | "tts" | "stt" | "audio" | "wake";

/** One installable file, as `downloads_search` reports it. */
export type DownloadFile = {
  repo: string;
  /** The path inside the repository; an install names this, not the file name. */
  path: string;
  name: string;
  size: number;
  /** Quantisation, quality or language -- what tells two files apart. */
  note: string;
  /**
   * The section of this kind's catalogue the file belongs to, when the kind has
   * more than one: an audio.cpp package's task (`speech`, `recognition`,
   * `wake word`) or which runtime a wake row needs. Absent for the kinds whose
   * rows are a single list, so the page can hide its section filter rather than
   * offer a control with one choice in it.
   */
  group?: string;
};

/** What `downloads-progress` carries while a file is arriving. */
export type DownloadProgress = {
  repo: string;
  path: string;
  received: number;
  total: number | null;
};

/** A file that is already on disk, as `downloads_installed` reports it. */
export type InstalledFile = {
  name: string;
  repo: string;
  path: string;
  size: number;
};

/** Where a job has got to. Mirrors `downloads::jobs::JobState`. */
export type JobState = "running" | "paused" | "done" | "stopped" | "failed";

/**
 * A download the app is doing or has done since it started.
 *
 * Held in the backend, not in a page: leaving the page must not lose the fact
 * that something is 40% through a two-gigabyte file.
 */
export type DownloadJob = {
  /** `<kind>/<repo>/<path>`, which is how a search row finds its own job. */
  id: string;
  kind: DownloadKind;
  repo: string;
  path: string;
  name: string;
  /** Where the finished file lands, so a model can be registered by path. */
  destination: string;
  state: JobState;
  received: number;
  total: number | null;
  /** Bytes per second, smoothed. Zero when nothing is moving. */
  speed: number;
  error: string | null;
};

/** A row is identified by what it installs, not by its position in the list. */
export function downloadKey(file: { repo: string; path: string }): string {
  return `${file.repo}/${file.path}`;
}

/** The id the backend gives a job, from the three things that name it. */
export function jobKey(kind: DownloadKind, repo: string, path: string): string {
  return `${kind}/${repo}/${path}`;
}
