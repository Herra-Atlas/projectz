/** The kinds of file the downloader installs. Mirrors `downloads::Kind`. */
export type DownloadKind = "llama" | "tts";

/** One installable file, as `downloads_search` reports it. */
export type DownloadFile = {
  repo: string;
  /** The path inside the repository; an install names this, not the file name. */
  path: string;
  name: string;
  size: number;
  /** Quantisation, quality or language -- what tells two files apart. */
  note: string;
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

/** A row is identified by what it installs, not by its position in the list. */
export function downloadKey(file: { repo: string; path: string }): string {
  return `${file.repo}/${file.path}`;
}
