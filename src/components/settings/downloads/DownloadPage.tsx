import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { LoaderCircle, Search } from "lucide-react";

import { SettingsPageTitle } from "../../SettingsSection";
import type { Notify } from "../../../features/notifications/types";
import { useDownloadSearch } from "../../../features/downloads/useDownloadSearch";
import { useDownloads } from "../../../features/downloads/useDownloads";
import {
  downloadKey,
  jobKey,
  type DownloadKind,
  type InstalledFile,
} from "../../../features/downloads/types";
import { DownloadRow } from "./DownloadRow";

/** Rows drawn before the first scroll, and added by each one after. */
const PAGE = 50;

const TITLES: Record<DownloadKind, string> = {
  llama: "Download a model",
  tts: "Download a voice",
  stt: "Download a recognition model",
  audio: "Download an audio model",
};

const PLACEHOLDERS: Record<DownloadKind, string> = {
  llama: "Search GGUF models — llama, qwen, mistral…",
  tts: "Search voices — amy, english, low…",
  stt: "Search models — tiny, base, large, parakeet…",
  audio: "Search audio.cpp models — kokoro, qwen3, parakeet…",
};

const HINTS: Record<DownloadKind, string> = {
  llama: "Each repository installs into its own folder, so a projector or tokenizer downloaded later sits beside the model.",
  tts: "A voice is its .onnx model plus the settings file beside it; both are installed together.",
  stt: "Whisper models first, then Parakeet. Both are read by the same engine — a parakeet.cpp GGUF is a different format and will not load.",
  audio: "audio.cpp packages from audio-cpp/audio.cpp-gguf; the family is read from the file name, and a package of an unknown family is refused at use.",
};

/**
 * The downloader.
 *
 * A sub-page of Local rather than a sidebar entry: it is reached from the engine
 * it installs for, and it installs for exactly one kind at a time -- which is why
 * the kind is a property of the page and not a control on it. A picker here would
 * be a second way to say what the back button already said.
 *
 * Results are drawn 50 at a time. The whole list is held in memory -- it is a few
 * hundred rows of text -- but drawing all of them at once is what makes a
 * settings page take a second to open, and scrolling is what asks for the rest.
 */
export function DownloadPage({
  kind,
  notify,
  onModelInstalled,
}: {
  kind: DownloadKind;
  notify: Notify;
  /** Called after a GGUF has been registered, so the list behind this page updates. */
  onModelInstalled: () => void;
}) {
  const [query, setQuery] = useState("");
  const { files, loading, error } = useDownloadSearch(kind, query);
  const [visible, setVisible] = useState(PAGE);

  const { jobs, start } = useDownloads((message) => notify("error", message));

  // What is already on disk, so a row says "Installed" instead of offering a
  // second copy. Read from the folder rather than from the job list: a file
  // downloaded in an earlier session is just as installed.
  const [installed, setInstalled] = useState<Set<string>>(new Set());
  const loadInstalled = useCallback(() => {
    void invoke<InstalledFile[]>("downloads_installed", { kind })
      .then((found) => setInstalled(new Set(found.map((file) => `${file.repo}/${file.name}`))))
      .catch(() => undefined);
  }, [kind]);
  useEffect(() => {
    loadInstalled();
  }, [loadInstalled]);
  useEffect(() => {
    let live = true;
    let stop: (() => void) | undefined;
    void listen("downloads-installed", loadInstalled).then((unlisten) => {
      if (live) stop = unlisten;
      else unlisten();
    });
    return () => {
      live = false;
      stop?.();
    };
  }, [loadInstalled]);

  // A GGUF is registered by the backend when its job finishes; this is what makes
  // the list behind the page re-read after that.
  const announced = useRef<Set<string>>(new Set());
  useEffect(() => {
    for (const job of jobs) {
      if (job.kind !== "llama" || job.state !== "done" || announced.current.has(job.id)) continue;
      announced.current.add(job.id);
      onModelInstalled();
    }
  }, [jobs, onModelInstalled]);

  // A new result set starts from the top, or a search made after scrolling would
  // open at row 200 of a list that now has eight.
  useEffect(() => {
    setVisible(PAGE);
  }, [files]);

  const sentinel = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const node = sentinel.current;
    if (!node || visible >= files.length) return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) setVisible((current) => current + PAGE);
      },
      // Ahead of the scroll, so the next page is there before it is needed.
      { rootMargin: "300px" },
    );
    observer.observe(node);
    return () => observer.disconnect();
  }, [files.length, visible]);

  const shown = files.slice(0, visible);

  return (
    <>
      <SettingsPageTitle>
        <h2 className="text-[17px] font-semibold tracking-tight">{TITLES[kind]}</h2>
      </SettingsPageTitle>

      <div className="mb-3 flex items-center gap-2 rounded-lg border border-[var(--line)] bg-[var(--panel)] px-3 focus-within:border-[var(--line-strong)]">
        <Search size={15} className="shrink-0 text-[var(--quiet)]" />
        <input
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder={PLACEHOLDERS[kind]}
          aria-label={PLACEHOLDERS[kind]}
          spellCheck={false}
          className="min-h-9 w-full bg-transparent text-[13px] text-[var(--text)] outline-none placeholder:text-[var(--quiet)]"
        />
        {loading && <LoaderCircle size={14} className="shrink-0 animate-spin text-[var(--quiet)]" />}
      </div>

      <p className="mb-3 text-[11px] leading-5 text-[var(--quiet)]">{HINTS[kind]}</p>

      {error !== null ? (
        <p className="rounded-md border border-[color-mix(in_srgb,var(--danger)_35%,var(--line))] px-3 py-2 text-[12px] text-[var(--danger)]">
          {error}
        </p>
      ) : files.length === 0 ? (
        // Only says "nothing found" once a search has answered. While the first
        // one is running the spinner above is the whole truth.
        !loading && <p className="px-1 py-6 text-sm text-[var(--muted)]">Nothing matched that search.</p>
      ) : (
        <>
          <p className="mb-1 px-1 text-[11px] text-[var(--quiet)]">
            {files.length} {files.length === 1 ? "result" : "results"}
            {files.length > visible ? ` · showing ${visible}` : ""}
          </p>
          <div className="divide-y divide-[var(--line)] border-y border-[var(--line)]">
            {shown.map((file) => {
              const key = downloadKey(file);
              const job = jobs.find((entry) => entry.id === jobKey(kind, file.repo, file.path));
              return (
                <DownloadRow
                  key={key}
                  file={file}
                  installed={installed.has(`${file.repo}/${file.name}`)}
                  job={job}
                  disabled={false}
                  onDownload={() => start(kind, file.repo, file.path)}
                />
              );
            })}
          </div>
          <div ref={sentinel} aria-hidden="true" className="h-px" />
          {visible < files.length && (
            <button
              type="button"
              onClick={() => setVisible((current) => current + PAGE)}
              className="mt-3 w-full rounded-md border border-[var(--line)] py-2 text-[12px] text-[var(--muted)] transition-colors hover:border-[var(--line-strong)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
            >
              Show {Math.min(PAGE, files.length - visible)} more
            </button>
          )}
        </>
      )}
    </>
  );
}
