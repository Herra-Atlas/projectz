import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from "react";
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
import { DownloadGroups, type DownloadSection } from "./DownloadGroups";
import { DownloadRow } from "./DownloadRow";

/** Rows drawn before the first scroll, and added by each one after. */
const PAGE = 50;

/**
 * The order the engine's own tasks read in.
 *
 * A catalogue sorted alphabetically would open on "recognition" because it
 * happens to sort before "speech"; the engine lists tts, asr, wake, and a reader
 * looking for a voice should find the voices first. Anything the table does not
 * name sorts after it, by label.
 */
const SECTION_ORDER = ["speech", "recognition", "wake word"];
function sectionRank(label: string): number {
  const at = SECTION_ORDER.indexOf(label);
  return at === -1 ? SECTION_ORDER.length : at;
}

const TITLES: Record<DownloadKind, string> = {
  llama: "Download a model",
  tts: "Download a voice",
  stt: "Download a recognition model",
  audio: "Download an audio model",
  wake: "Download a wake word model",
};

const PLACEHOLDERS: Record<DownloadKind, string> = {
  llama: "Search GGUF models — llama, qwen, mistral…",
  tts: "Search voices — amy, english, low…",
  stt: "Search models — tiny, base, large, parakeet…",
  audio: "Search audio.cpp models — kokoro, qwen3, parakeet…",
  wake: "Search wake words — alexa, nabu, keyword…",
};

const HINTS: Record<DownloadKind, string> = {
  llama: "Each repository installs into its own folder, so a projector or tokenizer downloaded later sits beside the model.",
  tts: "A voice is its .onnx model plus the settings file beside it; both are installed together.",
  stt: "Whisper models first, then Parakeet. Both are read by the same engine — a parakeet.cpp GGUF is a different format and will not load.",
  audio: "audio.cpp packages from audio-cpp/audio.cpp-gguf; the family is read from the file name, and a package of an unknown family is refused at use.",
  wake: "Two sources, one list: GGUF packages the audio.cpp engine runs, and openWakeWord heads from the community collection, which run in-process once the OpenWakeWord engine is installed.",
};

/**
 * The downloader.
 *
 * A sub-page of the engine it installs for, rather than a sidebar entry: the
 * kind is a property of the page, not a control on it. A picker here would be a
 * second way to say what the back button already said.
 *
 * Two of the kinds are more than one list -- audio.cpp serves speech,
 * recognition and wake-word packages, and the wake page draws two runtimes -- so
 * a search may be narrowed to a section. `group` is the section to open on,
 * named by the page that sent the user here: opening a download from Speech to
 * text should show the models that transcribe, not every file the repository
 * holds.
 *
 * Results are drawn 50 at a time. The whole list is held in memory -- it is a few
 * hundred rows of text -- but drawing all of them at once is what makes a
 * settings page take a second to open, and scrolling is what asks for the rest.
 */
export function DownloadPage({
  kind,
  group,
  notify,
  onModelInstalled,
}: {
  kind: DownloadKind;
  /** The section to open on, when the page that sent the user here named one. */
  group?: string;
  notify: Notify;
  /** Called after a GGUF has been registered, so the list behind this page updates. */
  onModelInstalled: () => void;
}) {
  const [query, setQuery] = useState("");
  const { files, loading, error } = useDownloadSearch(kind, query);
  const [visible, setVisible] = useState(PAGE);

  const [selected, setSelected] = useState<Set<string>>(() =>
    group === undefined ? new Set() : new Set([group]),
  );

  // The sections this search actually produced, counted rather than declared:
  // a family the repository no longer publishes must not be offered as a
  // section that would come back empty.
  const sections = useMemo<DownloadSection[]>(() => {
    const counts = new Map<string, number>();
    for (const file of files) {
      if (file.group === undefined) continue;
      counts.set(file.group, (counts.get(file.group) ?? 0) + 1);
    }
    return Array.from(counts, ([label, count]) => ({ label, count })).sort(
      (left, right) =>
        sectionRank(left.label) - sectionRank(right.label) ||
        left.label.localeCompare(right.label),
    );
  }, [files]);

  // Kept in a stable order with each section together, because the list is drawn
  // with a heading per section: unsorted, "speech, recognition, speech" would
  // draw the same heading twice. The sort is stable, so two rows in one section
  // keep the order the backend gave them.
  const matching = useMemo(() => {
    const narrowed =
      selected.size === 0
        ? files
        : files.filter((file) => file.group !== undefined && selected.has(file.group));
    return narrowed
      .slice()
      .sort((left, right) => sectionRank(left.group ?? "") - sectionRank(right.group ?? ""));
  }, [files, selected]);

  const toggleSection = (label: string) => {
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(label)) next.delete(label);
      else next.add(label);
      return next;
    });
  };

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
  // open at row 200 of a list that now has eight. A section change resets it for
  // the same reason: the narrowed list is a different list.
  useEffect(() => {
    setVisible(PAGE);
  }, [matching]);

  const sentinel = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const node = sentinel.current;
    if (!node || visible >= matching.length) return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) setVisible((current) => current + PAGE);
      },
      // Ahead of the scroll, so the next page is there before it is needed.
      { rootMargin: "300px" },
    );
    observer.observe(node);
    return () => observer.disconnect();
  }, [matching.length, visible]);

  const shown = matching.slice(0, visible);
  // Sections are drawn as headings only when the page actually has more than
  // one: a heading over the only list is a label doing no work.
  const divided = sections.length > 1;

  return (
    <>
      <SettingsPageTitle>
        <h2 className="text-[17px] font-semibold tracking-tight">{TITLES[kind]}</h2>
      </SettingsPageTitle>

      {/* The section filter sits beside the field rather than inside it: it is a
          different question ("which of these") from the one being typed. */}
      <div className="mb-3 flex items-center gap-2">
        <div className="flex min-w-0 flex-1 items-center gap-2 rounded-lg border border-[var(--line)] bg-[var(--panel)] px-3 focus-within:border-[var(--line-strong)]">
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
        {sections.length > 1 && (
          <DownloadGroups
            sections={sections}
            selected={selected}
            onToggle={toggleSection}
            onClear={() => setSelected(new Set())}
          />
        )}
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
      ) : matching.length === 0 ? (
        // A search that answered, and a section that excludes every answer it
        // gave, are two different facts: saying "nothing matched" would blame
        // the query for a filter the user set.
        <p className="px-1 py-6 text-sm text-[var(--muted)]">
          Nothing in the selected sections. <button type="button" onClick={() => setSelected(new Set())} className="text-[var(--accent)] underline-offset-2 hover:underline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]">Show all {files.length}</button>
        </p>
      ) : (
        <>
          <p className="mb-1 px-1 text-[11px] text-[var(--quiet)]">
            {matching.length}
            {selected.size > 0 && matching.length !== files.length ? ` of ${files.length}` : ""}{" "}
            {files.length === 1 ? "result" : "results"}
            {matching.length > visible ? ` · showing ${visible}` : ""}
          </p>
          <div className="divide-y divide-[var(--line)] border-y border-[var(--line)]">
            {shown.map((file, index) => {
              const key = downloadKey(file);
              const job = jobs.find((entry) => entry.id === jobKey(kind, file.repo, file.path));
              const heading = divided && file.group !== undefined && file.group !== shown[index - 1]?.group ? file.group : null;
              return (
                <Fragment key={key}>
                  {heading !== null && (
                    <p className="px-4 pb-1 pt-2.5 text-[11px] font-medium text-[var(--quiet)]">{heading}</p>
                  )}
                  <DownloadRow
                    file={file}
                    installed={installed.has(`${file.repo}/${file.name}`)}
                    job={job}
                    disabled={false}
                    onDownload={() => start(kind, file.repo, file.path)}
                  />
                </Fragment>
              );
            })}
          </div>
          <div ref={sentinel} aria-hidden="true" className="h-px" />
          {visible < matching.length && (
            <button
              type="button"
              onClick={() => setVisible((current) => current + PAGE)}
              className="mt-3 w-full rounded-md border border-[var(--line)] py-2 text-[12px] text-[var(--muted)] transition-colors hover:border-[var(--line-strong)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
            >
              Show {Math.min(PAGE, matching.length - visible)} more
            </button>
          )}
        </>
      )}
    </>
  );
}
