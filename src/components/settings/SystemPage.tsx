import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openPath } from "@tauri-apps/plugin-opener";
import { FolderOpen } from "lucide-react";

import { formatBytes } from "../../features/downloads/format";
import type { Notify } from "../../features/notifications/types";
import { HoldToDelete } from "../HoldToDelete";
import { SettingsPageTitle } from "../SettingsSection";

/** Mirrors `system::SystemInfo`. */
type SystemInfo = {
  version: string;
  buildDate: string;
  dataDir: string;
  modelsDir: string;
  enginesDir: string;
  voiceDir: string;
  databasePath: string;
  databaseBytes: number;
  modelsBytes: number;
  enginesBytes: number;
  voiceBytes: number;
};

/**
 * What this installation is, what it is using, and everything it can throw away.
 *
 * Every figure is measured when the page opens rather than cached: a size that
 * was true before you deleted a model is worse than no size at all.
 *
 * The reset section is a list of one job per row rather than a single switch,
 * because the reasons for clearing are different: reclaiming disk means the
 * models, being done with a project means the conversations, and starting over
 * means all of it. Each row asks once before it acts, and none of them act by
 * being on the page.
 */
export function SystemPage({ notify }: { notify: Notify }) {
  const [info, setInfo] = useState<SystemInfo | null>(null);
  const [working, setWorking] = useState<string | null>(null);

  const load = useCallback(() => {
    void invoke<SystemInfo>("system_info")
      .then(setInfo)
      .catch((reason: unknown) =>
        notify("error", `Could not read the system information: ${String(reason)}`),
      );
  }, [notify]);

  useEffect(() => {
    load();
  }, [load]);

  const run = useCallback(
    (id: string, command: string, done: string) => {
      setWorking(id);
      void invoke(command)
        .then(() => {
          notify("success", done);
          load();
        })
        .catch((reason: unknown) => notify("error", `Nothing was deleted: ${String(reason)}`))
        .finally(() => setWorking(null));
    },
    [load, notify],
  );

  const reveal = (path: string) => {
    void openPath(path).catch((reason: unknown) =>
      notify("error", `Could not open that folder: ${String(reason)}`),
    );
  };

  return (
    <>
      <SettingsPageTitle>
        <h2 className="text-[17px] font-semibold tracking-tight">System</h2>
      </SettingsPageTitle>

      <p className="mb-6 px-1 text-[12px] text-[var(--muted)]">
        Version {info?.version ?? "—"}, built {info?.buildDate ?? "—"}.
      </p>

      <section className="mb-8">
        <h3 className="mb-1.5 px-1 text-[11px] font-medium text-[var(--quiet)]">Storage</h3>
        <dl className="divide-y divide-[var(--line)] border-y border-[var(--line)]">
          <StorageRow
            label="Models"
            detail={info?.modelsDir ?? ""}
            size={info?.modelsBytes}
            onOpen={() => info && reveal(info.modelsDir)}
          />
          <StorageRow
            label="Engines"
            detail={info?.enginesDir ?? ""}
            size={info?.enginesBytes}
            onOpen={() => info && reveal(info.enginesDir)}
          />
          <StorageRow
            label="Voices"
            detail={info?.voiceDir ?? ""}
            size={info?.voiceBytes}
            onOpen={() => info && reveal(info.voiceDir)}
          />
          <StorageRow
            label="Database"
            detail={info?.databasePath ?? ""}
            size={info?.databaseBytes}
            onOpen={() => info && reveal(info.dataDir)}
          />
          <StorageRow
            label="App data"
            detail={info?.dataDir ?? ""}
            onOpen={() => info && reveal(info.dataDir)}
          />
        </dl>
      </section>

      <section>
        <h3 className="mb-1.5 px-1 text-[11px] font-medium text-[var(--quiet)]">Delete</h3>
        <div className="divide-y divide-[var(--line)] border-y border-[var(--line)]">
          <DeleteRow
            label="Conversations"
            detail="Every chat and its messages."
            action="Clear"
            busy={working === "sessions"}
            onDelete={() => run("sessions", "system_clear_sessions", "All conversations were cleared.")}
          />
          <DeleteRow
            label="Learned profile"
            detail="What the app learned about how you work, from Taste."
            action="Forget"
            busy={working === "taste"}
            onDelete={() => run("taste", "system_forget_taste", "The learned profile was forgotten.")}
          />
          <DeleteRow
            label="Models"
            detail="Every downloaded GGUF, with its folder."
            action="Delete"
            busy={working === "models"}
            onDelete={() => run("models", "system_clear_models", "All models were deleted.")}
          />
          <DeleteRow
            label="Engines"
            detail="llama.cpp and whisper.cpp builds, and the speech voices."
            action="Delete"
            busy={working === "engines"}
            onDelete={() => run("engines", "system_clear_engines", "All engines were deleted.")}
          />
          <DeleteRow
            label="Everything"
            detail="All of the above, plus the speech voices."
            action="Delete everything"
            danger
            busy={working === "all"}
            onDelete={() => run("all", "system_clear", "Everything the app downloaded was deleted.")}
          />
        </div>
        <p className="mt-2 px-1 text-[11px] leading-5 text-[var(--quiet)]">
          Providers, keys and settings are kept.
        </p>
      </section>
    </>
  );
}

/** A folder, what is in it, and a way to get there. */
function StorageRow({
  label,
  detail,
  size,
  onOpen,
}: {
  label: string;
  detail: string;
  size?: number;
  onOpen: () => void;
}) {
  return (
    <div className="flex items-center gap-3 px-4 py-2.5">
      <dt className="w-24 shrink-0 text-[12px] text-[var(--muted)]">{label}</dt>
      <dd className="min-w-0 flex-1">
        <p className="truncate text-[12px] text-[var(--text)]" title={detail}>
          {detail || "—"}
        </p>
      </dd>
      <span className="w-16 shrink-0 text-right text-[11px] tabular-nums text-[var(--quiet)]">
        {size === undefined ? "—" : formatBytes(size)}
      </span>
      <button
        type="button"
        onClick={onOpen}
        aria-label={`Open ${label} folder`}
        title="Open folder"
        className="grid size-8 shrink-0 place-items-center rounded-md text-[var(--quiet)] hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
      >
        <FolderOpen size={14} />
      </button>
    </div>
  );
}

/**
 * One thing that can be deleted.
 *
 * The confirmation is a two-second hold, shared with every other delete in
 * settings: a click is what a misclick produces, so a click does not delete.
 */
function DeleteRow({
  label,
  detail,
  action,
  danger = false,
  busy,
  onDelete,
}: {
  label: string;
  detail: string;
  action: string;
  danger?: boolean;
  busy: boolean;
  onDelete: () => void;
}) {
  return (
    <div className="flex items-center gap-3 px-4 py-2.5">
      <div className="min-w-0 flex-1">
        <h4 className="text-[13px] text-[var(--text)]">{label}</h4>
        <p className="text-[11px] leading-5 text-[var(--quiet)]">{detail}</p>
      </div>
      <HoldToDelete
        onConfirm={onDelete}
        disabled={busy}
        label={`${action} ${label.toLowerCase()}`}
        title={`${action} ${label.toLowerCase()} — hold to confirm`}
        className={`min-h-8 shrink-0 rounded-md px-2.5 text-[12px] transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)] disabled:opacity-50 ${
          danger
            ? "border border-[color-mix(in_srgb,var(--danger)_40%,var(--line))] font-medium text-[var(--danger)] hover:bg-[color-mix(in_srgb,var(--danger)_10%,transparent)]"
            : "text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]"
        }`}
      >
        {busy ? "Working…" : action}
      </HoldToDelete>
    </div>
  );
}
