import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  FileCode2,
  FilePlus2,
  FileText,
  LoaderCircle,
  Save,
  Trash2,
} from "lucide-react";
import { HoldToDelete } from "../HoldToDelete";
import {
  fileSize,
  type SkillFile,
} from "../../features/skills/types";
import type { Notify } from "../../features/notifications/types";

/** The prefix `skills_read_file` uses for a file it cannot show as text. */
const BINARY_MARKER = "is not UTF-8 text";

/**
 * The files beside a skill's `SKILL.md`.
 *
 * **A list and one editor, not a tree browser.** A skill is a handful of small
 * files in three conventional folders; a tree with disclosure triangles would add
 * navigation to something that already fits on screen. Clicking a row opens it
 * below.
 *
 * **Binary files are readable as facts, not as text.** The backend answers with a
 * short description instead of bytes, and saving that description back would
 * destroy the asset -- so a file it could not decode is shown read-only and the
 * save control is withheld. The alternative is a buffer that looks like content.
 */
export default function SkillFilesPanel({
  slug,
  files,
  onChanged,
  notify,
}: {
  slug: string;
  files: SkillFile[];
  /** Re-reads the skill, so sizes and the script badge stay honest. */
  onChanged: () => Promise<void>;
  notify: Notify;
}) {
  const [selected, setSelected] = useState<string | null>(null);
  const [content, setContent] = useState("");
  const [draftPath, setDraftPath] = useState("");
  const [creating, setCreating] = useState(false);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [binary, setBinary] = useState(false);

  useEffect(() => {
    if (!selected) {
      setContent("");
      setBinary(false);
      return;
    }
    let current = true;
    setLoading(true);
    invoke<string>("skills_read_file", { slug, path: selected })
      .then((text) => {
        if (!current) return;
        setContent(text);
        setBinary(text.includes(BINARY_MARKER));
      })
      .catch((reason) => {
        if (!current) return;
        setContent("");
        notify("error", `Could not read ${selected}: ${String(reason)}`);
      })
      .finally(() => {
        if (current) setLoading(false);
      });
    return () => {
      current = false;
    };
  }, [slug, selected, notify]);

  const openNew = async () => {
    const path = draftPath.trim();
    if (!path) return;
    try {
      // Created empty before it is opened, so the editor reads a real file
      // rather than asking for one that does not exist yet.
      await invoke("skills_write_file", { slug, path, content: "" });
      await onChanged();
      setSelected(path);
      setContent("");
      setBinary(false);
      setDraftPath("");
      setCreating(false);
    } catch (reason) {
      notify("error", String(reason));
    }
  };

  const save = async () => {
    if (!selected || binary) return;
    setSaving(true);
    try {
      await invoke("skills_write_file", { slug, path: selected, content });
      await onChanged();
      notify("success", `${selected} saved`);
    } catch (reason) {
      notify("error", String(reason));
    } finally {
      setSaving(false);
    }
  };

  const remove = async (path: string) => {
    try {
      await invoke("skills_delete_file", { slug, path });
      if (selected === path) setSelected(null);
      await onChanged();
      notify("success", `${path} deleted`);
    } catch (reason) {
      notify("error", String(reason));
    }
  };

  const selectedFile = files.find((file) => file.path === selected || file.path === draftPath);

  return (
    <section className="rounded-xl border border-[var(--line)] bg-[var(--panel)]">
      <header className="flex items-center justify-between gap-2 border-b border-[var(--line)] px-4 py-3">
        <div>
          <h3 className="text-[12.5px] font-medium text-[var(--text)]">Files</h3>
          <p className="mt-0.5 text-[11px] text-[var(--quiet)]">
            Scripts, references and assets the skill can use.
          </p>
        </div>
        <button
          type="button"
          onClick={() => setCreating((open) => !open)}
          className="grid size-8 shrink-0 place-items-center rounded-md text-[var(--quiet)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
          aria-label="Add a file"
          title="Add a file"
        >
          <FilePlus2 size={15} />
        </button>
      </header>

      {files.length === 0 && !creating ? (
        <p className="px-4 py-4 text-[11.5px] leading-5 text-[var(--quiet)]">
          No supporting files. Add a script under <code>scripts/</code>, or notes under{" "}
          <code>references/</code>.
        </p>
      ) : (
        <ul className="divide-y divide-[var(--line)]">
          {files.map((file) => {
            const active = file.path === selected;
            return (
              <li key={file.path} className="flex items-center gap-1 pr-1">
                <button
                  type="button"
                  onClick={() => setSelected(file.path)}
                  className={`flex min-w-0 flex-1 items-center gap-2 px-3 py-2 text-left transition-colors hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)] ${
                    active ? "bg-[var(--raised)]" : ""
                  }`}
                >
                  {file.is_script ? (
                    <FileCode2 size={13} className="shrink-0 text-[var(--accent)]" aria-hidden />
                  ) : (
                    <FileText size={13} className="shrink-0 text-[var(--quiet)]" aria-hidden />
                  )}
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-[12px] text-[var(--text)]" title={file.path}>
                      {file.path}
                    </span>
                    <span className="block text-[10.5px] text-[var(--quiet)]">
                      {fileSize(file.size)}
                      {file.is_script ? " · script" : ""}
                    </span>
                  </span>
                </button>
                <HoldToDelete
                  onConfirm={() => void remove(file.path)}
                  label={`Delete ${file.path}`}
                  className="grid size-7 shrink-0 place-items-center rounded-md text-[var(--quiet)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--danger)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
                >
                  <Trash2 size={13} />
                </HoldToDelete>
              </li>
            );
          })}
        </ul>
      )}

      {creating && (
        <div className="flex gap-2 border-t border-[var(--line)] p-3">
          <input
            autoFocus
            value={draftPath}
            onChange={(event) => setDraftPath(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                void openNew();
              }
            }}
            placeholder="scripts/run.ps1"
            spellCheck={false}
            className="h-8 min-w-0 flex-1 rounded-md border border-[var(--line)] bg-[var(--rail)] px-2.5 text-[11.5px] text-[var(--text)] outline-none placeholder:text-[var(--quiet)] focus:border-[var(--accent)]"
          />
          <button
            type="button"
            onClick={() => void openNew()}
            disabled={!draftPath.trim()}
            className="min-h-8 rounded-md border border-[var(--line)] px-2.5 text-[11.5px] text-[var(--text)] transition-colors hover:bg-[var(--raised)] disabled:opacity-40 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
          >
            Add
          </button>
        </div>
      )}

      {selected && (
        <div className="border-t border-[var(--line)] p-3">
          <div className="mb-2 flex items-center justify-between gap-2">
            <span className="truncate font-mono text-[11px] text-[var(--muted)]" title={selected}>
              {selected}
            </span>
            <span className="flex items-center gap-2">
              {!binary && (
                <button
                  type="button"
                  onClick={() => void save()}
                  disabled={saving}
                  className="inline-flex min-h-7 items-center gap-1.5 rounded-md bg-[var(--accent)] px-2.5 text-[11px] font-semibold text-[var(--accent-ink)] transition-opacity hover:opacity-90 disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
                >
                  {saving ? <LoaderCircle size={12} className="animate-spin" /> : <Save size={12} />}
                  Save
                </button>
              )}
            </span>
          </div>
          {loading ? (
            <p className="flex items-center gap-2 py-6 text-[11.5px] text-[var(--quiet)]">
              <LoaderCircle size={12} className="animate-spin" /> Reading…
            </p>
          ) : binary ? (
            <p className="rounded-md bg-[var(--rail)] px-3 py-2.5 text-[11.5px] leading-5 text-[var(--muted)]">
              {content}
            </p>
          ) : (
            <textarea
              value={content}
              onChange={(event) => setContent(event.target.value)}
              rows={12}
              spellCheck={false}
              className="w-full resize-y rounded-md border border-[var(--line)] bg-[var(--rail)] px-3 py-2 font-mono text-[11.5px] leading-5 text-[var(--text)] outline-none focus:border-[var(--accent)]"
            />
          )}
          {selectedFile?.is_script && (
            <p className="mt-2 text-[10.5px] leading-4 text-[var(--quiet)]">
              A script runs only after “Allow scripts” is on for this skill.
            </p>
          )}
        </div>
      )}
    </section>
  );
}
