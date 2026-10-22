import { useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Download, LoaderCircle, Plus, Search, Sparkles, X } from "lucide-react";
import { ANCHOR_ATTRIBUTE, FlashHighlight, MARKABLE_ATTRIBUTE, SettingsPageTitle } from "./SettingsSection";
import SkillCard from "./skills/SkillCard";
import InstallSkillDialog from "./skills/InstallSkillDialog";
import SkillsEmptyState from "./skills/SkillsEmptyState";
import { useSkillList } from "../features/skills/useSkills";
import type { Skill } from "../features/skills/types";
import type { Notify } from "../features/notifications/types";

/**
 * The skills wall: every folder the app knows, grouped by family.
 *
 * **It does not edit.** Writing a skill happens on its own page, so this screen
 * only asks to go there. That keeps the form's draft state out of the list and
 * means the list has one job: let a reader find the skill they mean.
 *
 * **Search above the groups, not a filter bar.** A user with twenty skills is
 * looking for one by name or by what it is for; a one-line field is the whole
 * affordance. It appears only once the list is long enough that scanning beats
 * typing, so a first-run screen stays quiet.
 *
 * **Disabled skills stay visible**, drawn on the quieter surface. Hiding them
 * would make "where did that skill go" answerable only by turning a filter back
 * on.
 */
export default function SkillsPage({
  notify,
  onOpenForm,
}: {
  notify: Notify;
  /** Opens the create/edit page: the skill to edit, or `null` for a new one. */
  onOpenForm: (skill: Skill | null) => void;
}) {
  const { skills, loading, failed, reload } = useSkillList();
  const [query, setQuery] = useState("");
  const [installing, setInstalling] = useState(false);

  const on = skills.filter((skill) => skill.enabled).length;
  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return skills;
    return skills.filter((skill) =>
      [skill.name, skill.slug, skill.description ?? "", skill.skill_type ?? ""]
        .join(" ")
        .toLowerCase()
        .includes(needle),
    );
  }, [skills, query]);
  const grouped = groupByType(filtered);

  const toggle = async (skill: Skill) => {
    try {
      await invoke("skills_set_enabled", { slug: skill.slug, enabled: !skill.enabled });
      await reload();
    } catch (reason) {
      notify("error", `Could not change this skill: ${String(reason)}`);
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
              : `${skills.length} skill${skills.length === 1 ? "" : "s"} · ${on} offered to the agent`}
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
            onClick={() => onOpenForm(null)}
            className="inline-flex min-h-9 items-center gap-1.5 rounded-lg bg-[var(--accent)] px-3.5 text-[12.5px] font-semibold text-[var(--accent-ink)] transition-opacity hover:opacity-90 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
          >
            <FlashHighlight anchor="skills.list" />
            <Plus size={14} /> New skill
          </button>
        </div>
      </SettingsPageTitle>

      {loading ? (
        <p className="flex items-center gap-2 text-sm text-[var(--muted)]">
          <LoaderCircle size={14} className="animate-spin" /> Reading skills…
        </p>
      ) : failed ? (
        <p className="rounded-lg border border-[var(--line)] bg-[var(--panel)] px-4 py-3 text-sm text-[var(--danger)]">
          Skills could not be read: {failed}
        </p>
      ) : skills.length === 0 ? (
        <SkillsEmptyState onCreate={() => onOpenForm(null)} />
      ) : (
        <>
          {skills.length > 5 && (
            <div className="mb-5 flex items-center gap-2">
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
            <div className="space-y-7">
              {grouped.map(([label, group]) => (
                <section key={label}>
                  <div className="mb-3 flex items-baseline gap-2">
                    <h3 className="text-[11px] font-semibold uppercase tracking-wide text-[var(--quiet)]">
                      {label}
                    </h3>
                    <span className="text-[10.5px] text-[var(--quiet)]">{group.length}</span>
                  </div>
                  <div className="grid gap-3 sm:grid-cols-2">
                    {group.map((skill) => (
                      <SkillCard
                        key={skill.slug}
                        skill={skill}
                        onOpen={onOpenForm}
                        onToggle={(target) => void toggle(target)}
                      />
                    ))}
                  </div>
                </section>
              ))}
            </div>
          )}

          <p className="mt-7 flex items-center gap-1.5 text-[10.5px] text-[var(--quiet)]">
            <Sparkles size={11} aria-hidden />
            Skills live in <code>skills/</code> beside the app's data. Edit them here or in any
            text editor.
          </p>
        </>
      )}

      {installing && (
        <InstallSkillDialog
          onClose={() => setInstalling(false)}
          onInstalled={(skill) => {
            setInstalling(false);
            void reload();
            notify("success", `${skill.name} installed`);
            onOpenForm(skill);
          }}
        />
      )}
    </div>
  );
}

/**
 * Groups by the free-text label.
 *
 * Unlabelled skills are collected under one heading rather than each floating on
 * its own, because most skills have no label and a section per skill would be a
 * heading per card. Unlabelled sorts last: it is the absence of a choice, not a
 * choice.
 */
function groupByType(skills: Skill[]): [string, Skill[]][] {
  const groups = new Map<string, Skill[]>();
  for (const skill of skills) {
    const label = skill.skill_type?.trim() || "Unlabelled";
    groups.set(label, [...(groups.get(label) ?? []), skill]);
  }
  return [...groups.entries()].sort(([left], [right]) => {
    if (left === "Unlabelled") return 1;
    if (right === "Unlabelled") return -1;
    return left.localeCompare(right);
  });
}
