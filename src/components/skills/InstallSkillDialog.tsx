import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Download, LoaderCircle, ShieldCheck, X } from "lucide-react";
import type { Skill } from "../../features/skills/types";

/**
 * Installs a skill from a link.
 *
 * **A dialog, because it is one decision.** Pasting a link and pressing Install
 * is the whole interaction; a whole settings page would give that decision more
 * room than it has content. The dialog states the one thing the user needs to
 * know before installing: what arrives is a folder they can read and edit, with
 * whatever scripts it carries visible in the same tree.
 *
 * **The app fetches, not the agent.** This calls a Tauri command from a button.
 * Nothing the model can reach downloads and unpacks executable material; the
 * reason is in `skills/install.rs`.
 */
export default function InstallSkillDialog({
  onInstalled,
  onClose,
}: {
  onInstalled: (skill: Skill) => void;
  onClose: () => void;
}) {
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const install = async () => {
    const trimmed = url.trim();
    if (!trimmed) {
      setError("Paste a link first.");
      return;
    }
    setBusy(true);
    setError("");
    try {
      const skill = await invoke<Skill>("skills_install", { url: trimmed });
      onInstalled(skill);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 grid place-items-center bg-black/60 p-4"
      onKeyDown={(event) => {
        if (event.key === "Escape" && !busy) onClose();
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="skill-install-title"
        className="w-full max-w-md rounded-xl border border-[var(--line)] bg-[var(--panel)] p-5 shadow-2xl"
      >
        <div className="flex items-start justify-between gap-3">
          <div className="flex items-center gap-2">
            <span className="grid size-8 place-items-center rounded-lg bg-[var(--raised)] text-[var(--muted)]">
              <Download size={15} />
            </span>
            <h3 id="skill-install-title" className="text-sm font-medium text-[var(--text)]">
              Install a skill
            </h3>
          </div>
          <button
            type="button"
            onClick={onClose}
            disabled={busy}
            aria-label="Close"
            className="grid size-8 place-items-center rounded-md text-[var(--quiet)] hover:bg-[var(--raised)] hover:text-[var(--text)] disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
          >
            <X size={15} />
          </button>
        </div>

        <p className="mt-2 text-xs leading-5 text-[var(--muted)]">
          Paste a GitHub repository or a link to a <code className="text-[var(--text)]">.zip</code>{" "}
          containing a <code className="text-[var(--text)]">SKILL.md</code>. The folder is saved
          to your skills directory and can be edited afterwards.
        </p>

        <label className="mt-4 block text-[12px] text-[var(--muted)]">
          Link
          <input
            autoFocus
            value={url}
            onChange={(event) => setUrl(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && !busy) void install();
            }}
            placeholder="https://github.com/owner/skill-repo"
            spellCheck={false}
            className="mt-1.5 h-9 w-full rounded-lg border border-[var(--line)] bg-[var(--rail)] px-3 text-[12.5px] text-[var(--text)] outline-none placeholder:text-[var(--quiet)] focus:border-[var(--accent)]"
          />
        </label>

        <p className="mt-3 flex items-start gap-2 rounded-lg bg-[var(--rail)] px-3 py-2 text-[11px] leading-4 text-[var(--quiet)]">
          <ShieldCheck size={13} className="mt-0.5 shrink-0 text-[var(--muted)]" aria-hidden />
          Nothing runs on its own. It installs as a folder you can read, and any scripts it carries
          are only run when you open them.
        </p>

        {error && (
          <p role="alert" className="mt-3 text-[11.5px] leading-5 text-[var(--danger)]">
            {error}
          </p>
        )}

        <div className="mt-4 flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            disabled={busy}
            className="min-h-9 rounded-lg px-3 text-[12.5px] text-[var(--muted)] transition-colors hover:bg-[var(--raised)] disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={() => void install()}
            disabled={busy}
            className="inline-flex min-h-9 items-center gap-1.5 rounded-lg bg-[var(--accent)] px-3.5 text-[12.5px] font-semibold text-[var(--accent-ink)] transition-opacity hover:opacity-90 disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
          >
            {busy ? <LoaderCircle size={14} className="animate-spin" /> : <Download size={14} />}
            {busy ? "Installing…" : "Install"}
          </button>
        </div>
      </div>
    </div>
  );
}
