import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  AlertCircle,
  Check,
  ChevronLeft,
  Download,
  LoaderCircle,
  ShieldCheck,
  X,
} from "lucide-react";
import HelpMark from "../HelpMark";
import type {
  Discovery,
  FoundSkill,
  InstallProgress,
  InstallReport,
  Skill,
} from "../../features/skills/types";

/** The dialog is one decision with at most one intermediate choice. */
type Stage = "input" | "resolving" | "picking" | "installing" | "done";

/**
 * Whether a link names skills or is a file to install directly.
 *
 * A `.zip` is checked first because a GitHub archive URL also names a
 * repository; pointing it at the archive is what the user asked for.
 */
const looksLikeRepository = (value: string) => {
  const trimmed = value.trim();
  if (!trimmed || /\.zip(\?|#|$)/i.test(trimmed)) return false;
  if (/^https?:\/\//i.test(trimmed)) {
    return /^https?:\/\/(www\.)?github\.com\//i.test(trimmed);
  }
  return (
    /^[^/\s:]+\/[^/\s]+/.test(trimmed) &&
    !trimmed.startsWith(".") &&
    !trimmed.includes("\\")
  );
};

const filesLabel = (skill: FoundSkill) => {
  const files =
    skill.file_count === 0
      ? "SKILL.md only"
      : `${skill.file_count} file${skill.file_count === 1 ? "" : "s"}`;
  return skill.has_scripts ? `${files} · scripts` : files;
};

/**
 * The `?` beside an error, answering "and now what?".
 *
 * The message says what failed; the hint says which door to try next. Keyed on
 * the same words the backend uses, because those strings are the only thing the
 * dialog knows about why a request stopped.
 */
const fixFor = (message: string) => {
  const lower = message.toLowerCase();
  if (lower.includes("quota") || lower.includes("rate limit") || lower.includes("60 requests")) {
    return "GitHub's anonymous limit is 60 requests an hour. Wait for the reset time in the message, or paste the skill's own .zip link. This dialog never uses Node or npx.";
  }
  if (lower.includes("no skills") || lower.includes("no skill")) {
    return "That link names no SKILL.md in a published folder. Paste the skill's own folder URL (…/tree/<branch>/skills/<name>), the SKILL.md URL, or its .zip.";
  }
  if (lower.includes("already installed")) {
    return "A folder with that name is already in your skills directory. Open it in the Files panel, then delete or rename it and install again.";
  }
  if (lower.includes("larger") || lower.includes("size") || lower.includes("limit is")) {
    return "A skill is capped at 8 MB per file and 16 MB in total. Paste the skill's own folder link or its .zip rather than a whole-repository archive.";
  }
  if (lower.includes("404") || lower.includes("no such file") || lower.includes("no longer")) {
    return "The branch or path in that link no longer exists. Copy the folder URL from GitHub again, or paste the skill's .zip.";
  }
  if (lower.includes("npx") || lower.includes("not installed") || lower.includes("not on path")) {
    return "This dialog needs neither Node nor npx. If this came from a terminal command, install Node so it lands on the registry PATH, then restart the app.";
  }
  return "Check the branch and path in the link. A folder link (…/tree/<branch>/<path>), a SKILL.md URL, or a .zip is the most direct route; a repository link shows a picker instead.";
};

/**
 * Installs skills from a link.
 *
 * **A dialog, because it is one decision.** Paste `owner/repo`, a GitHub URL or
 * a `.zip`. A repository is resolved to the skills it publishes and shown as a
 * short list — the same choice `npx skills add` offers — and each selected
 * folder is installed on its own. A direct folder link preselects its skill.
 *
 * **The app fetches, not the agent.** This calls a Tauri command from a button.
 * Nothing the model can reach downloads and unpacks executable material; the
 * reason is in `skills/install.rs`.
 *
 * **Progress is an event, not a spinner.** A repository can take a few seconds
 * to list and a skill can hold many files; the backend emits `skills-install`
 * with what it is doing so the bar moves and the message names the skill.
 */
export default function InstallSkillDialog({
  onInstalled,
  onClose,
}: {
  onInstalled: (skills: Skill[]) => void;
  onClose: () => void;
}) {
  const [url, setUrl] = useState("");
  const [stage, setStage] = useState<Stage>("input");
  const [discovery, setDiscovery] = useState<Discovery | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [progress, setProgress] = useState<InstallProgress | null>(null);
  const [report, setReport] = useState<InstallReport | null>(null);
  const [error, setError] = useState("");
  const busy = stage === "resolving" || stage === "installing";

  useEffect(() => {
    const pending = listen<InstallProgress>("skills-install", (event) => {
      setProgress(event.payload);
    });
    return () => {
      pending.then((unlisten) => unlisten());
    };
  }, []);

  const percent = useMemo(() => {
    if (!progress || progress.total <= 0) return 0;
    return Math.min(100, Math.round((progress.done / progress.total) * 100));
  }, [progress]);

  const allSelected = discovery
    ? discovery.skills.length > 0 &&
      discovery.skills.every((skill) => selected.has(skill.id))
    : false;

  const describe = async () => {
    const trimmed = url.trim();
    if (!trimmed) {
      setError("Paste a link first.");
      return;
    }
    setError("");
    setReport(null);

    if (!looksLikeRepository(trimmed)) {
      // A .zip has no picker: it holds one skill by the rules in install.rs.
      setStage("installing");
      setProgress({ phase: "downloading", message: "Downloading the archive", done: 0, total: 0 });
      try {
        const skill = await invoke<Skill>("skills_install", { url: trimmed });
        onInstalled([skill]);
      } catch (reason) {
        setError(String(reason));
        setStage("input");
      }
      return;
    }

    setStage("resolving");
    setProgress({ phase: "reading", message: "Reading the repository", done: 0, total: 0 });
    try {
      const found = await invoke<Discovery>("skills_discover", { url: trimmed });
      if (found.skills.length === 0) {
        throw new Error(`No skills found in ${found.owner}/${found.repo}.`);
      }
      setDiscovery(found);
      setSelected(
        new Set(found.skills.length === 1 ? [found.skills[0].id] : []),
      );
      setStage("picking");
    } catch (reason) {
      setError(String(reason));
      setStage("input");
    }
  };

  const install = async () => {
    if (!discovery || selected.size === 0) return;
    setError("");
    setReport(null);
    setStage("installing");
    setProgress({ phase: "downloading", message: "Starting the download", done: 0, total: 0 });
    try {
      const outcome = await invoke<InstallReport>("skills_install_selected", {
        url: url.trim(),
        reference: discovery.reference,
        skills: [...selected],
      });
      if (outcome.failed.length === 0) {
        onInstalled(outcome.installed);
        return;
      }
      setReport(outcome);
      setStage("done");
    } catch (reason) {
      setError(String(reason));
      setStage("picking");
    }
  };

  const toggle = (id: string) =>
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const finish = () => {
    if (report && report.installed.length > 0) onInstalled(report.installed);
    else onClose();
  };

  return (
    <div
      className="fixed inset-0 z-50 grid place-items-center bg-black/60 p-4"
      onKeyDown={(event) => {
        if (event.key !== "Escape" || busy) return;
        // A finished list may hold installed skills; closing it the same way the
        // Done button does keeps the Skills page in step.
        if (stage === "done") finish();
        else onClose();
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
            <h3
              id="skill-install-title"
              className="flex items-center gap-1.5 text-sm font-medium text-[var(--text)]"
            >
              <span>Install a skill</span>
              <HelpMark hint="Reads a GitHub repository's published skills without cloning it, then downloads only the folders you tick — into ProjectZ's own skills directory. No Node or npx, and it never writes to .claude or .agents." />
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

        {stage === "input" && (
          <>
            <p className="mt-2 text-xs leading-5 text-[var(--muted)]">
              Paste <code className="text-[var(--text)]">owner/repo</code>, a GitHub repository or a
              folder inside one, or a link to a{" "}
              <code className="text-[var(--text)]">.zip</code> containing a{" "}
              <code className="text-[var(--text)]">SKILL.md</code>. A repository shows the skills it
              publishes so you can choose.
            </p>

            <label className="mt-4 block text-[12px] text-[var(--muted)]">
              <span className="flex items-center gap-1.5">
                Link
                <HelpMark hint="owner/repo, a GitHub URL, a /tree/ or /blob/ folder link, or a .zip. Copying 'npx skills add owner/repo' works too — the command is stripped. A repository shows a picker; a folder link goes straight to its skill." />
              </span>
              <input
                autoFocus
                value={url}
                onChange={(event) => setUrl(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter" && !busy) void describe();
                }}
                placeholder="heygen-com/hyperframes"
                spellCheck={false}
                className="mt-1.5 h-9 w-full rounded-lg border border-[var(--line)] bg-[var(--rail)] px-3 text-[12.5px] text-[var(--text)] outline-none placeholder:text-[var(--quiet)] focus:border-[var(--accent)]"
              />
            </label>

            <p className="mt-3 flex items-start gap-2 rounded-lg bg-[var(--rail)] px-3 py-2 text-[11px] leading-4 text-[var(--quiet)]">
              <ShieldCheck size={13} className="mt-0.5 shrink-0 text-[var(--muted)]" aria-hidden />
              Nothing runs on its own. A skill installs as a folder you can read, and any scripts it
              carries are only run when you open them.
            </p>
          </>
        )}

        {stage === "picking" && discovery && (
          <>
            <div className="mt-2 flex items-baseline justify-between gap-3">
              <p className="truncate text-xs text-[var(--muted)]">
                <span className="text-[var(--text)]">
                  {discovery.owner}/{discovery.repo}
                </span>{" "}
                · {discovery.skills.length} skill{discovery.skills.length === 1 ? "" : "s"}
              </p>
              <button
                type="button"
                onClick={() =>
                  setSelected(
                    allSelected
                      ? new Set()
                      : new Set(discovery.skills.map((skill) => skill.id)),
                  )
                }
                className="shrink-0 text-[11px] text-[var(--accent)] hover:underline"
              >
                {allSelected ? "Select none" : "Select all"}
              </button>
            </div>

            {discovery.truncated && (
              <p className="mt-2 text-[11px] leading-4 text-[var(--danger)]">
                GitHub truncated this repository&rsquo;s file list, so a skill may be missing. A
                folder link names one directly.
              </p>
            )}

            <div className="mt-2 max-h-64 overflow-y-auto rounded-lg border border-[var(--line)] p-1">
              {discovery.skills.map((skill) => (
                <label
                  key={skill.id}
                  className="flex cursor-pointer items-start gap-2.5 rounded-md px-2 py-2 hover:bg-[var(--raised)]"
                >
                  <input
                    type="checkbox"
                    checked={selected.has(skill.id)}
                    onChange={() => toggle(skill.id)}
                    className="mt-0.5 size-3.5 shrink-0 accent-[var(--accent)]"
                  />
                  <span className="min-w-0 flex-1">
                    <span className="flex items-baseline justify-between gap-2">
                      <span className="truncate text-[12.5px] font-medium text-[var(--text)]">
                        {skill.name}
                      </span>
                      <span className="shrink-0 text-[10.5px] text-[var(--quiet)]">
                        {filesLabel(skill)}
                      </span>
                    </span>
                    {skill.description && (
                      <span className="mt-0.5 line-clamp-2 block text-[11px] leading-4 text-[var(--muted)]">
                        {skill.description}
                      </span>
                    )}
                  </span>
                </label>
              ))}
            </div>
          </>
        )}

        {(stage === "resolving" || stage === "installing") && (
          <div className="mt-3">
            <div className="h-1.5 overflow-hidden rounded-full bg-[var(--rail)]">
              <div
                className={`h-full rounded-full bg-[var(--accent)] transition-[width] duration-200 ${
                  progress && progress.total > 0 ? "" : "w-1/3 animate-pulse"
                }`}
                style={progress && progress.total > 0 ? { width: `${percent}%` } : undefined}
              />
            </div>
            <p className="mt-2 flex items-center gap-2 text-[11.5px] text-[var(--muted)]">
              <LoaderCircle size={12} className="shrink-0 animate-spin" />
              <span className="truncate">{progress?.message ?? "Working…"}</span>
              {progress && progress.total > 0 && (
                <span className="ml-auto shrink-0 text-[var(--quiet)]">
                  {progress.done}/{progress.total}
                </span>
              )}
            </p>
          </div>
        )}

        {stage === "done" && report && (
          <div className="mt-3 space-y-2">
            {report.installed.length > 0 && (
              <p className="flex items-center gap-2 rounded-lg bg-[var(--rail)] px-3 py-2 text-[11.5px] text-[var(--text)]">
                <Check size={13} className="shrink-0 text-[var(--accent)]" />
                {report.installed.length} skill{report.installed.length === 1 ? "" : "s"} installed
              </p>
            )}
            {report.failed.map((failure) => (
              <p
                key={failure.id}
                className="flex items-start gap-2 text-[11.5px] leading-5 text-[var(--danger)]"
              >
                <AlertCircle size={13} className="mt-0.5 shrink-0" />
                <span className="min-w-0">
                  <span className="text-[var(--text)]">
                    {failure.id || "the repository root"}
                  </span>{" "}
                  — {failure.message}
                </span>
              </p>
            ))}
          </div>
        )}

        {error && (
          <div
            role="alert"
            className="mt-3 flex items-start gap-1.5 text-[11.5px] leading-5 text-[var(--danger)]"
          >
            <span className="min-w-0">{error}</span>
            <span className="mt-0.5 shrink-0">
              <HelpMark hint={fixFor(error)} />
            </span>
          </div>
        )}

        <div className="mt-4 flex justify-end gap-2">
          {stage === "picking" && (
            <button
              type="button"
              onClick={() => {
                setStage("input");
                setDiscovery(null);
                setSelected(new Set());
                setProgress(null);
              }}
              className="inline-flex min-h-9 items-center gap-1 rounded-lg px-3 text-[12.5px] text-[var(--muted)] transition-colors hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
            >
              <ChevronLeft size={14} /> Back
            </button>
          )}
          {stage === "input" && (
            <button
              type="button"
              onClick={onClose}
              className="min-h-9 rounded-lg px-3 text-[12.5px] text-[var(--muted)] transition-colors hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
            >
              Cancel
            </button>
          )}
          {stage === "done" && (
            <button
              type="button"
              onClick={finish}
              className="min-h-9 rounded-lg px-3 text-[12.5px] text-[var(--muted)] transition-colors hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
            >
              Close
            </button>
          )}
          {(stage === "input" || stage === "picking") && (
            <button
              type="button"
              onClick={() => (stage === "input" ? void describe() : void install())}
              disabled={stage === "picking" && selected.size === 0}
              className="inline-flex min-h-9 items-center gap-1.5 rounded-lg bg-[var(--accent)] px-3.5 text-[12.5px] font-semibold text-[var(--accent-ink)] transition-opacity hover:opacity-90 disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
            >
              <Download size={14} />
              {stage === "input"
                ? "Continue"
                : `Install ${selected.size} skill${selected.size === 1 ? "" : "s"}`}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
