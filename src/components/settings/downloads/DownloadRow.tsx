import { Check, Download, LoaderCircle } from "lucide-react";

import { formatBytes } from "../../../features/downloads/format";
import type { DownloadFile } from "../../../features/downloads/types";

/**
 * One installable file.
 *
 * The name leads because it is what a reader scans for; the repository sits under
 * it because several repositories publish a file of the same name and only the
 * repository says which one this is. The size is on the right, next to the button,
 * because "is this the download I want" is usually a question about size.
 */
export function DownloadRow({
  file,
  installed,
  busy,
  disabled,
  progress,
  onInstall,
}: {
  file: DownloadFile;
  installed: boolean;
  busy: boolean;
  disabled: boolean;
  /** Fraction complete, or undefined while the size is unknown. */
  progress: number | undefined;
  onInstall: () => void;
}) {
  const percent = progress === undefined ? null : Math.round(progress * 100);

  return (
    <div className="relative px-4 py-2.5">
      <div className="flex items-center gap-3">
        <div className="min-w-0 flex-1">
          <h4 className="truncate text-[13px] font-medium text-[var(--text)]" title={file.path}>
            {file.name}
          </h4>
          <p className="truncate text-[11px] text-[var(--quiet)]">
            {[file.repo, file.note].filter(Boolean).join(" · ")}
          </p>
        </div>

        <span className="hidden shrink-0 text-[11px] tabular-nums text-[var(--muted)] sm:block">
          {formatBytes(file.size)}
        </span>

        <button
          type="button"
          onClick={onInstall}
          disabled={disabled || installed}
          aria-label={installed ? `${file.name} installed` : `Download ${file.name}`}
          className={`inline-flex min-h-8 w-[104px] shrink-0 items-center justify-center gap-1.5 rounded-md px-2.5 text-[12px] font-medium transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)] ${
            installed
              ? "text-[var(--quiet)]"
              : busy
                ? "border border-[var(--line)] text-[var(--muted)]"
                : "border border-[var(--line)] text-[var(--text)] hover:border-[var(--line-strong)] hover:bg-[var(--raised)] disabled:opacity-40"
          }`}
        >
          {installed ? (
            <>
              <Check size={13} />
              <span>Installed</span>
            </>
          ) : busy ? (
            <>
              <LoaderCircle size={13} className="animate-spin" />
              <span>{percent === null ? "Starting" : `${percent}%`}</span>
            </>
          ) : (
            <>
              <Download size={13} />
              <span>Download</span>
            </>
          )}
        </button>
      </div>

      {/* A line under the row rather than a bar inside the button: the number is
          already in the button, and the width is what says "this will take a
          while" without a second control competing for the pointer. */}
      {busy && <div className="absolute inset-x-0 bottom-0 h-px bg-[var(--line)]"><div className="h-px bg-[var(--accent)] transition-[width] duration-200" style={{ width: `${percent ?? 0}%` }} /></div>}
    </div>
  );
}
