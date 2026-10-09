import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ChevronDown, ChevronRight, Download, Link2, LoaderCircle, RefreshCw, Trash2, Zap } from "lucide-react";
import type { Notify } from "../features/notifications/types";

type Engine = {
  id: string;
  version: string;
  updated_at: string;
  asset_name: string;
  size_bytes: number;
  installed: boolean;
  /**
   * False for an engine whose release has dropped out of the catalog.
   *
   * Such a row is kept because the files are on disk and still usable, but the
   * two things a catalog row normally carries -- a download URL and a publish
   * date -- are gone, so it must not offer a Download button or a size.
   */
  available: boolean;
  /**
   * `llama.cpp` for a verified catalog build, `custom` for one installed from
   * a pasted release link.
   *
   * The backend resolves this rather than the frontend inferring it from the id:
   * a custom engine's id is `{owner}-{repo}-{tag}`, so which of the two it is
   * cannot be read off the name the way the catalog's prefix can.
   */
  source: string;
};
type EnginesSettingsPageProps = { onChanged: () => void; notify: Notify };
type DownloadProgress = { id: string; downloaded: number; total: number; complete?: boolean };

/** How many rows are shown before "Show more" is needed. */
const VISIBLE_ENGINES = 3;

/** What a row says about where it came from. */
const CUSTOM_SOURCE = "custom";

const formatSize = (bytes: number) => `${(bytes / (1024 ** 2)).toFixed(0)} MB`;
const formatProgress = ({ downloaded, total }: DownloadProgress) => {
  const percentage = total > 0 ? Math.min(100, Math.floor((downloaded / total) * 100)) : 0;
  return `${percentage}% · ${formatSize(downloaded)} / ${formatSize(total)}`;
};
const formatDate = (date: string) => {
  const parsed = new Date(date);
  return Number.isNaN(parsed.getTime()) ? date : parsed.toLocaleDateString();
};

/**
 * One release row: version, date and size on the left, actions on the right.
 *
 * Split out of the page so the three states -- downloading, installed, and
 * installed-but-no-longer-published -- are each readable on their own rather
 * than as one nested conditional.
 */
function EngineRow({
  engine,
  progress,
  working,
  busy,
  onInstall,
  onUninstall,
}: {
  engine: Engine;
  progress?: DownloadProgress;
  /** True for the row being installed or removed right now. */
  working: boolean;
  /** True while *any* row is working, which is what disables the others. */
  busy: boolean;
  onInstall: (engine: Engine) => Promise<void>;
  onUninstall: (engine: Engine) => Promise<void>;
}) {
  // A catalog row carries a publish date and a size. A synthesised one for an
  // engine that has aged out has neither, so it says so rather than printing
  // "1 Jan 1970 · 0 MB" -- which reads as a corrupt record rather than as an
  // engine the release list no longer mentions.
  const details = engine.available
    ? `${formatDate(engine.updated_at)} · ${formatSize(engine.size_bytes)}`
    : "No longer listed upstream";
  const custom = engine.source === CUSTOM_SOURCE;

  return (
    <div className="flex min-h-9 flex-wrap items-center gap-x-3 gap-y-2 py-1.5 pr-2">
      <div className="min-w-[180px] flex-1">
        <p className="text-xs font-medium text-[var(--muted)]">
          {engine.version}
          {/* Says where a build came from, because a fork is not a llama.cpp
              release and the two look identical in a list of versions. Stated
              rather than inferred: "unverified" is the honest description of a
              link that carried no checksum to check against, and it is the only
              thing distinguishing this row from a verified one below it. */}
          {custom && <span className="ml-2 text-[10px] text-[var(--quiet)]">from link · unverified</span>}
        </p>
        <p className="mt-0.5 text-[11px] text-[var(--quiet)]">{details}</p>
      </div>
      <div className="flex items-center gap-1.5">
        {!engine.installed ? (
          <button type="button" onClick={() => void onInstall(engine)} disabled={busy} className="inline-flex min-h-8 items-center gap-1.5 rounded-md px-2 text-xs font-medium text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)] disabled:opacity-50">
            <Download size={13} />
            {working ? progress?.complete ? "Verifying…" : progress ? formatProgress(progress) : "Starting…" : "Download"}
          </button>
        ) : (
          <button type="button" onClick={() => void onUninstall(engine)} disabled={busy} className="grid size-8 place-items-center rounded-md text-[var(--quiet)] hover:bg-[var(--raised)] hover:text-[var(--danger)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)] disabled:opacity-40" aria-label={`Uninstall engine ${engine.version}`} title="Uninstall engine"><Trash2 size={14} /></button>
        )}
      </div>
    </div>
  );
}

export default function EnginesSettingsPage({ onChanged, notify }: EnginesSettingsPageProps) {
  const [engines, setEngines] = useState<Engine[]>([]);
  const [expanded, setExpanded] = useState(false);
  /** The link section's own expansion, independent of the catalog's. */
  const [linkOpen, setLinkOpen] = useState(false);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [workingId, setWorkingId] = useState<string | null>(null);
  const [downloadProgress, setDownloadProgress] = useState<Record<string, DownloadProgress>>({});
  /**
   * Only the catalog-load failure stays on this page.
   *
   * It is the one error that *is* the page: when the list cannot be read there
   * is no list, and an empty panel with the reason in the corner would look like
   * a bug. Everything else -- an install, a select, an uninstall -- happens to a
   * list that is still on screen, so it goes to `notify` where it is reported
   * once rather than in a line that scrolls away under a page of engines.
   */
  const [loadError, setLoadError] = useState("");
  /**
   * Whether the full release list is shown.
   *
   * Three rows is enough to see that builds are arriving; the catalog holds
   * thirty, which as a wall of near-identical `Download` rows buries the ones
   * the user actually installed. Collapsing back to three on refresh is
   * deliberate -- a list that stays expanded after the catalog changes is
   * showing a set of rows that may no longer exist.
   */
  const [showAll, setShowAll] = useState(false);
  /**
   * A GitHub release link, typed rather than pasted-and-gone.
   *
   * Held in state so a failed install leaves the text where it was: the reason a
   * link is refused is almost always that the wrong address was copied, and a
   * field cleared on failure would make that a retype rather than a correction.
   */
  const [link, setLink] = useState("");
  /**
   * True while a pasted link is being downloaded.
   *
   * Separate from `workingId` because there is no row for it yet -- the engine
   * does not exist until the archive has been fetched, verified and expanded --
   * so a row cannot be the thing that says it is happening. The notification and
   * this flag are what report it.
   */
  const [installingLink, setInstallingLink] = useState(false);

  const installFromLink = async () => {
    const trimmed = link.trim();
    if (!trimmed || installingLink) return;
    setInstallingLink(true);
    // Keyed by the link rather than by an engine id, because there is no engine
    // id until this succeeds -- and a stable key is what turns "Downloading…" into
    // "installed" in one row instead of leaving the first message behind.
    const key = "engine-link";
    notify("loading", "Downloading engine…", key);
    try {
      await invoke("local_engines_custom_install", { link: trimmed });
      setLink("");
      notify("success", "Engine installed", key);
      await loadEngines(false);
      onChanged();
    } catch (reason) {
      notify("error", `Engine could not be installed: ${String(reason)}`, key);
    } finally {
      setInstallingLink(false);
    }
  };

  const loadEngines = useCallback(async (refresh: boolean) => {
    if (refresh) setRefreshing(true);
    else setLoading(true);
    // A refreshed catalog is a different list, so an expansion applied to the
    // previous one no longer means anything.
    if (refresh) setShowAll(false);
    try {
      const items = await invoke<Engine[]>("local_engines_list", { refresh });
      setEngines(items);
      setLoadError("");
    } catch (reason) {
      setLoadError(`Engine catalog could not be loaded: ${String(reason)}`);
    } finally {
      setLoading(false);
      setRefreshing(false);
    }
  }, []);

  useEffect(() => { void loadEngines(false); }, [loadEngines]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void listen<DownloadProgress>("engine-download-progress", (event) => {
      setDownloadProgress((current) => ({ ...current, [event.payload.id]: event.payload }));
    }).then((stop) => { unlisten = stop; });
    return () => unlisten?.();
  }, []);

  const installEngine = async (engine: Engine) => {
    setWorkingId(engine.id);
    // Keyed by engine id, so the loading row and its outcome are the same row.
    // Two engines could not be downloaded at once anyway, but the key is what
    // makes "Downloading… 40%" become "installed" rather than becoming a second
    // row while the first is still claiming to be in progress.
    const key = `engine:${engine.id}`;
    notify("loading", `Downloading ${engine.version} Vulkan engine…`, key);
    try {
      await invoke("local_engines_install", { id: engine.id });
      setDownloadProgress((current) => { const next = { ...current }; delete next[engine.id]; return next; });
      notify("success", `${engine.version} installed`, key);
      await loadEngines(false);
      onChanged();
    } catch (reason) {
      setDownloadProgress((current) => { const next = { ...current }; delete next[engine.id]; return next; });
      notify("error", `Engine could not be installed: ${String(reason)}`, key);
    } finally {
      setWorkingId(null);
    }
  };

  const uninstallEngine = async (engine: Engine) => {
    setWorkingId(engine.id);
    try {
      await invoke("local_engines_uninstall", { id: engine.id });
      notify("success", `${engine.version} removed`, `engine:${engine.id}`);
      await loadEngines(false);
    } catch (reason) {
      notify("error", `Engine could not be removed: ${String(reason)}`, `engine:${engine.id}`);
    } finally {
      setWorkingId(null);
    }
  };

  /**
   * Split by origin, so a fork is never drawn as though llama.cpp published it.
   *
   * A separate section rather than a filter on one list: the two sets have
   * different trust -- one is checksum-verified against GitHub's own digest and
   * the other is whatever a link pointed at -- and putting them in one list
   * would let the sorted order imply that distinction had been made.
   */
  const customEngines = engines.filter((engine) => engine.source === CUSTOM_SOURCE);
  const catalogEngines = engines.filter((engine) => engine.source !== CUSTOM_SOURCE);
  const visible = showAll ? catalogEngines : catalogEngines.slice(0, VISIBLE_ENGINES);
  const hiddenCount = catalogEngines.length - visible.length;
  // Counted across the whole catalog, not only the visible rows: an engine below
  // the fold used to report "0 installed" while it sat on disk taking up space.
  //
  // Scoped to `catalogEngines`, and that is the part that was wrong. Counting
  // every installed engine made this header claim the fork listed under "From a
  // link" further down the page -- two builds, one of which is not a llama.cpp
  // release and is already reported by its own section.
  const installedCount = catalogEngines.filter((engine) => engine.installed).length;
  const summary = loading
    ? "Checking installed versions…"
    : `${installedCount} installed · ${catalogEngines.length} versions available`;

  return (
    <section>
      <div className="mb-6 flex flex-wrap items-center justify-between gap-3 border-b border-[var(--line)] pb-5">
        <h2 className="text-[17px] font-semibold tracking-tight">Engines</h2>
        <button type="button" onClick={() => void loadEngines(true)} disabled={refreshing || loading} className="inline-flex min-h-9 items-center gap-1.5 rounded-lg border border-[var(--line)] px-3 text-[13px] text-[var(--muted)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)] disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"><RefreshCw size={14} className={refreshing ? "animate-spin" : ""} />Refresh catalog</button>
      </div>

      <div className="divide-y divide-[var(--line)] overflow-hidden rounded-lg border border-[var(--line)] bg-[var(--panel)]">
        <article>
          <div className="flex min-h-[68px] items-center gap-2 px-4 py-3">
            <button type="button" onClick={() => setExpanded((current) => !current)} aria-expanded={expanded} aria-label={`${expanded ? "Collapse" : "Expand"} llama.cpp Vulkan engines`} title={`${expanded ? "Collapse" : "Expand"} llama.cpp Vulkan engines`} className="grid size-9 shrink-0 place-items-center rounded-md text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"><Zap size={16} /></button>
            <div className="min-w-0 flex-1"><h3 className="truncate text-sm font-medium">llama.cpp Vulkan</h3><p className="truncate text-xs text-[var(--quiet)]">{summary}</p></div>
            <span className="hidden shrink-0 text-[11px] text-[var(--muted)] sm:block">Windows x64</span>
            <span className="grid size-9 shrink-0 place-items-center text-[var(--quiet)]" aria-hidden="true">{expanded ? <ChevronDown size={15} /> : <ChevronRight size={15} />}</span>
          </div>

          {expanded && <div className="mb-3 ml-[2.75rem] border-l border-[var(--line)] py-1 pl-3">
            {loading ? <div className="flex min-h-9 items-center gap-2 text-xs text-[var(--muted)]"><LoaderCircle size={14} className="animate-spin" />Loading engine releases</div> : loadError && engines.length === 0
              // Names the actual failure rather than asserting a platform
              // limitation. The previous copy said Vulkan downloads "are
              // currently supported on Windows x64" for every error, so a dropped
              // connection or a rate-limited API read as a definitive statement
              // about the machine -- and told the user to retry something that was
              // never the problem. Stays inline: with no list to draw, this text
              // is the page rather than a message about it.
              ? <p role="alert" className="py-2 text-xs text-[var(--danger)]">{loadError}</p>
              : catalogEngines.length === 0 ? <p className="py-2 text-xs text-[var(--quiet)]">No compatible Vulkan releases found. Refresh the catalog to check again.</p> : <div className="divide-y divide-[var(--line)]">
              {visible.map((engine) => <EngineRow key={engine.id} engine={engine} progress={downloadProgress[engine.id]} working={workingId === engine.id} busy={workingId !== null} onInstall={installEngine} onUninstall={uninstallEngine} />)}
              {/* Only when there is genuinely more to see. */}
              {(hiddenCount > 0 || showAll) && (
                <button type="button" onClick={() => setShowAll((current) => !current)} aria-expanded={showAll} className="flex min-h-9 w-full items-center gap-2 py-1.5 text-xs text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]">
                  {showAll ? <ChevronDown size={13} className="shrink-0" /> : <ChevronRight size={13} className="shrink-0" />}
                  {showAll ? "Show fewer" : `Show ${hiddenCount} more`}
                </button>
              )}
            </div>}
          </div>}
        </article>

        {/* Always present rather than shown only once something is installed:
            the link field is how a fork gets added, and a section that appears
            after the fact is a section someone who has no fork yet never finds. */}
        <article>
          <div className="flex min-h-[68px] flex-wrap items-center gap-2 px-4 py-3">
            <button type="button" onClick={() => setLinkOpen((current) => !current)} aria-expanded={linkOpen} aria-label={`${linkOpen ? "Collapse" : "Expand"} engines from a link`} title={`${linkOpen ? "Collapse" : "Expand"} engines from a link`} className="grid size-9 shrink-0 place-items-center rounded-md text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"><Link2 size={16} /></button>
            <div className="min-w-0 flex-1">
              <h3 className="truncate text-sm font-medium">From a link</h3>
              <p className="truncate text-xs text-[var(--quiet)]">{customEngines.length === 0 ? "Add a llama.cpp fork this catalog does not carry" : `${customEngines.length} installed`}</p>
            </div>
            <span className="hidden shrink-0 text-[11px] text-[var(--quiet)] sm:block">Unverified</span>
            <span className="grid size-9 shrink-0 place-items-center text-[var(--quiet)]" aria-hidden="true">{linkOpen ? <ChevronDown size={15} /> : <ChevronRight size={15} />}</span>
          </div>

          {linkOpen && <div className="mb-3 ml-[2.75rem] border-l border-[var(--line)] py-1 pl-3">
            {customEngines.length > 0 && <div className="divide-y divide-[var(--line)] pb-1">
              {customEngines.map((engine) => <EngineRow key={engine.id + "-custom"} engine={engine} working={workingId === engine.id} busy={workingId !== null} onInstall={installEngine} onUninstall={uninstallEngine} />)}
            </div>}
            <div className="flex flex-wrap items-center gap-2 py-1.5">
              <label className="flex min-h-9 min-w-0 flex-1 items-center gap-2 rounded-md border border-[var(--line)] px-2.5 text-[var(--muted)] focus-within:border-[color-mix(in_srgb,var(--accent)_45%,var(--line))]">
                <Link2 size={14} aria-hidden="true" className="shrink-0" />
                <span className="sr-only">GitHub release download link</span>
                <input
                  value={link}
                  onChange={(event) => setLink(event.target.value)}
                  onKeyDown={(event) => { if (event.key === "Enter") void installFromLink(); }}
                  placeholder="github.com/owner/repo/releases/download/tag/llama-…-win-vulkan-x64.zip"
                  spellCheck={false}
                  autoComplete="off"
                  className="min-w-0 flex-1 bg-transparent py-2 text-xs text-[var(--text)] outline-none placeholder:text-[var(--quiet)]"
                />
              </label>
              <button type="button" onClick={() => void installFromLink()} disabled={!link.trim() || installingLink || workingId !== null} className="inline-flex min-h-9 shrink-0 items-center gap-1.5 rounded-md border border-[var(--line)] px-3 text-xs font-medium text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)] disabled:opacity-50">
                {installingLink ? <LoaderCircle size={13} className="animate-spin" /> : <Download size={13} />}
                {installingLink ? "Installing…" : "Install"}
              </button>
            </div>
            {/* Says the one thing that differs from the catalog above, in the
                place someone is about to click. A link carries no checksum --
                GitHub publishes a digest per release asset and not on its
                download address -- so these bytes cannot be checked against
                anything, and a user installing a fork is entitled to know that
                before the download rather than after. */}
            <p className="pb-1 text-[11px] text-[var(--quiet)]">A release file link from GitHub. ProjectZ cannot check it against a published checksum, so it is installed unverified.</p>
          </div>}
        </article>
      </div>
      {/* The spinner stays inline because it tracks a specific row: `workingId`
          names the engine being downloaded, and that fact is only visible on the
          page. The notification reports the same work, but a row at the top of the
          stack cannot say which row of thirty it is talking about. */}
      {workingId && <p role="status" className="mt-3 flex items-center gap-2 text-xs text-[var(--accent)]"><LoaderCircle size={13} className="animate-spin" />Working on this engine…</p>}
    </section>
  );
}
