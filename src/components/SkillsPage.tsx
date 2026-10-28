import { useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Download, Folder, LoaderCircle, Plus, Search, X } from "lucide-react";
import { ANCHOR_ATTRIBUTE, FlashHighlight, MARKABLE_ATTRIBUTE, SettingsPageTitle } from "./SettingsSection";
import SkillRow from "./skills/SkillRow";
import InstallSkillDialog from "./skills/InstallSkillDialog";
import SkillsEmptyState from "./skills/SkillsEmptyState";
import NameInput from "./rightPanel/tabs/NameInput";
import { createSkill, useSkillList } from "../features/skills/useSkills";
import type { Skill } from "../features/skills/types";
import type { Notify } from "../features/notifications/types";

/**
 * The skills list: one row per folder.
 *
 * **It does not edit.** A skill is a folder, and the app already has a folder
 * editor -- the right panel's Files view. Opening a row hands the folder to that
 * view and closes this window, so a skill is written in the tree the user already
 * knows, with create, rename, drag and preview working on it, rather than in a
 * form that could only ever do the one file.
 *
 * **Search above the list, not a filter bar.** A user with twenty skills is
 * looking for one by name or by what it is for; a one-line field is the whole
 * affordance. It appears only once the list is long enough that scanning beats
 * typing, so a first-run screen stays quiet.
 */
export default function SkillsPage({
  notify,
  onOpenSkill,
}: {
  notify: Notify;
  /** Opens a skill's folder in the right panel. */
  onOpenSkill: (slug: string) => void;
}) {
  const { skills, loading, failed, reload } = useSkillList();
  const [query, setQuery] = useState("");
  const [installing, setInstalling] = useState(false);
  const [busy, setBusy] = useState(false);
  /** The name field is open, waiting for a skill's name. */
  const [naming, setNaming] = useState(false);

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return skills;
    return skills.filter((skill) =>
      [skill.name, skill.slug, skill.description ?? ""].join(" ").toLowerCase().includes(needle),
    );
  }, [skills, query]);

  /**
   * Makes a new skill and opens it.
   *
   * **The name is typed in the page, not in a browser prompt.** `window.prompt`
   * is not implemented by the webview the app runs in -- it returns `null` and
   * the press looked like a button that did nothing. The field is the tree's own
   * naming control (`NameInput`), so Enter commits, Escape cancels and clicking
   * away keeps the name, exactly as creating a file or renaming a row behaves.
   */
  const create = async (name: string) => {
    setNaming(false);
    setBusy(true);
    try {
      const skill = await createSkill(name);
      await reload();
      onOpenSkill(skill.slug);
    } catch (reason) {
      notify("error", `Could not create the skill: ${String(reason)}`);
    } finally {
      setBusy(false);
    }
  };

  const remove = async (skill: Skill) => {
    if (!window.confirm(`Delete the skill "${skill.name}" and everything in its folder?`)) return;
    try {
      await invoke("skills_delete", { slug: skill.slug });
      notify("success", `${skill.name} deleted`);
      await reload();
    } catch (reason) {
      notify("error", `Could not delete this skill: ${String(reason)}`);
    }
  };

  return (
    <div>
      <SettingsPageTitle>
        <div className="min-w-0">
          <h2 className="text-[17px] font-semibold tracking-tight">Skills</h2>
          <p className="mt-0.5 text-[11.5px] text-[var(--quiet)]">
            {skills.length === 0
              ? "Reusable methods the agent reads when they apply."
              : `${skills.length} skill${skills.length === 1 ? "" : "s"} · every one offered to the agent`}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <button
            type="button"
            {...{ [ANCHOR_ATTRIBUTE]: "skills.install", [MARKABLE_ATTRIBUTE]: "" }}
            onClick={() => setInstalling(true)}
            className="inline-flex min-h-9 items-center gap-1.5 rounded-lg border border-[var(--line)] bg-[var(--panel)] px-3 text-[12.5px] font-medium text-[var(--muted)] transition-colors hover:border-[var(--line-strong)] hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
          >
            <FlashHighlight anchor="skills.install" />
            <Download size={14} /> Install
          </button>
          <button
            type="button"
            {...{ [ANCHOR_ATTRIBUTE]: "skills.list", [MARKABLE_ATTRIBUTE]: "" }}
            onClick={() => setNaming(true)}
            disabled={busy}
            className="inline-flex min-h-9 items-center gap-1.5 rounded-lg bg-[var(--accent)] px-3.5 text-[12.5px] font-semibold text-[var(--accent-ink)] transition-opacity hover:opacity-90 disabled:opacity-60 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
          >
            <FlashHighlight anchor="skills.list" />
            {busy ? <LoaderCircle size={14} className="animate-spin" /> : <Plus size={14} />} New skill
          </button>
        </div>
      </SettingsPageTitle>

      {/* The name lives in the page, beside the list it is about to join -- the
          same inline row the Files tree uses. It stays open while the folder is
          being made, so the spinner on the button has something to belong to. */}
      {naming && (
        <div className="mb-4 flex items-center gap-2 rounded-lg border border-[var(--line)] bg-[var(--panel)] px-3 py-2">
          <Folder size={14} className="shrink-0 text-[var(--muted)]" />
          <span className="shrink-0 text-[12px] text-[var(--quiet)]">New skill</span>
          <NameInput
            initialValue="new-skill"
            onCommit={(name) => void create(name)}
            onCancel={() => setNaming(false)}
          />
        </div>
      )}

      {loading ? (
        <p className="flex items-center gap-2 text-sm text-[var(--muted)]">
          <LoaderCircle size={14} className="animate-spin" /> Reading skills…
        </p>
      ) : failed ? (
        <p className="rounded-lg border border-[var(--line)] bg-[var(--panel)] px-4 py-3 text-sm text-[var(--danger)]">
          Skills could not be read: {failed}
        </p>
      ) : skills.length === 0 ? (
        <SkillsEmptyState onCreate={() => setNaming(true)} />
      ) : (
        <>
          {skills.length > 5 && (
            <div className="mb-4 flex items-center gap-2">
              <label className="relative block w-full max-w-xs">
                <Search
                  size={13}
                  aria-hidden
                  className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-[var(--quiet)]"
                />
                <input
                  value={query}
                  onChange={(event) => setQuery(event.target.value)}
                  placeholder="Filter skills…"
                  className="h-8 w-full rounded-lg border border-[var(--line)] bg-[var(--panel)] pl-8 pr-8 text-[12px] text-[var(--text)] outline-none placeholder:text-[var(--quiet)] focus:border-[var(--accent)]"
                />
                {query && (
                  <button
                    type="button"
                    onClick={() => setQuery("")}
                    aria-label="Clear the filter"
                    className="absolute right-1.5 top-1/2 grid size-6 -translate-y-1/2 place-items-center rounded text-[var(--quiet)] hover:text-[var(--text)]"
                  >
                    <X size={12} />
                  </button>
                )}
              </label>
              {query && (
                <span className="text-[11px] text-[var(--quiet)]">
                  {filtered.length} match{filtered.length === 1 ? "" : "es"}
                </span>
              )}
            </div>
          )}

          {filtered.length === 0 ? (
            <p className="rounded-lg border border-dashed border-[var(--line)] px-4 py-8 text-center text-[12.5px] text-[var(--quiet)]">
              Nothing matches “{query}”.
            </p>
          ) : (
            <div className="overflow-hidden rounded-lg border border-[var(--line)]">
              {filtered.map((skill, index) => (
                <SkillRow
                  key={skill.slug}
                  skill={skill}
                  index={index}
                  onOpen={(target) => onOpenSkill(target.slug)}
                  onDelete={(target) => void remove(target)}
                />
              ))}
            </div>
          )}
        </>
      )}

      {installing && (
        <InstallSkillDialog
          onClose={() => setInstalling(false)}
          onInstalled={(installed) => {
            setInstalling(false);
            void reload();
            if (installed.length === 1) {
              notify("success", `${installed[0].name} installed`);
              onOpenSkill(installed[0].slug);
            } else if (installed.length > 1) {
              notify("success", `${installed.length} skills installed`);
            }
          }}
        />
      )}
    </div>
  );
}
