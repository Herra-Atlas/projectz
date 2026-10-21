import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Check, Cpu, Download, LoaderCircle, Mic } from "lucide-react";

import { formatBytes } from "../../../features/downloads/format";
import type { InstalledFile } from "../../../features/downloads/types";
import type { Notify } from "../../../features/notifications/types";
import { useStt } from "../../../features/stt/useStt";

/**
 * The recognition half of the Local page.
 *
 * Two things to install, and they are separate on purpose: the engine is a
 * whisper.cpp build with the Vulkan backend (16 MB packed, one per machine), and
 * a model is the file that actually transcribes (75 MB and up, one per size or
 * language). Someone with several models still installs the engine once.
 *
 * Nothing downloads on its own -- the engine is a button, the models are a page.
 */
export function SpeechRecognitionCategory({
  onDownload,
  notify,
}: {
  onDownload: () => void;
  notify: Notify;
}) {
  const { status, progress, installing, install } = useStt((message) => notify("error", message));
  const [models, setModels] = useState<InstalledFile[]>([]);

  // Re-read on mount: coming back from the download page is a fresh mount, which
  // is what makes a model appear here without a second event.
  useEffect(() => {
    let live = true;
    void invoke<InstalledFile[]>("downloads_installed", { kind: "stt" })
      .then((next) => {
        if (live) setModels(next);
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, []);

  const engine = status?.engineInstalled === true;
  const percent = progress === null ? null : Math.round(progress * 100);

  return (
    <article>
      <div className="flex min-h-[68px] items-center gap-2 px-4 py-3">
        <span className="grid size-9 shrink-0 place-items-center rounded-md text-[var(--muted)]">
          <Cpu size={16} />
        </span>
        <div className="min-w-0 flex-1">
          <h3 className="truncate text-sm font-medium">Whisper</h3>
          <p className="truncate text-xs text-[var(--quiet)]">
            {status === null
              ? "Checking…"
              : engine
                ? `${status.engineVersion} · ${models.length === 1 ? "1 model" : `${models.length} models`}`
                : "Engine not installed · Vulkan"}
          </p>
        </div>
        {engine ? (
          <button
            type="button"
            onClick={onDownload}
            className="inline-flex min-h-8 shrink-0 items-center gap-1.5 rounded-md border border-[var(--line)] px-2.5 text-[12px] text-[var(--text)] transition-colors hover:border-[var(--line-strong)] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
          >
            <Download size={13} />
            <span>Download model</span>
          </button>
        ) : (
          <button
            type="button"
            onClick={install}
            disabled={installing || status === null}
            className="inline-flex min-h-8 w-[104px] shrink-0 items-center justify-center gap-1.5 rounded-md border border-[var(--line)] px-2.5 text-[12px] font-medium text-[var(--text)] transition-colors hover:border-[var(--line-strong)] hover:bg-[var(--raised)] disabled:opacity-40 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
          >
            {installing ? <LoaderCircle size={13} className="animate-spin" /> : <Download size={13} />}
            <span>{installing ? (percent === null ? "Starting" : `${percent}%`) : "Install"}</span>
          </button>
        )}
      </div>

      <div className="mb-3 ml-[2.75rem] border-l border-[var(--line)] py-1 pl-3">
        {engine && (
          <div className="flex min-h-9 items-center gap-3 py-1.5 pr-2">
            <Check size={14} className="shrink-0 text-[var(--accent)]" />
            <div className="min-w-0 flex-1">
              <h4 className="truncate text-xs font-medium text-[var(--muted)]">Speech engine</h4>
              <p className="truncate text-[11px] text-[var(--quiet)]">
                {status?.engineVersion} · Vulkan, with a CPU fallback
              </p>
            </div>
          </div>
        )}

        {models.map((file) => (
          <div key={file.path} className="flex min-h-9 items-center gap-3 py-1.5 pr-2" title={file.path}>
            <Mic size={14} className="shrink-0 text-[var(--quiet)]" />
            <div className="min-w-0 flex-1">
              <h4 className="truncate text-xs font-medium text-[var(--muted)]">{file.name}</h4>
              <p className="truncate text-[11px] text-[var(--quiet)]">
                {file.repo} · {formatBytes(file.size)}
              </p>
            </div>
          </div>
        ))}

        {engine && models.length === 0 && (
          <p className="py-2 text-xs text-[var(--quiet)]">
            No models yet. Download one to start transcribing.
          </p>
        )}
      </div>
    </article>
  );
}
