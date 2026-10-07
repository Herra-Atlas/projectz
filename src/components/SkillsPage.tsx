import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { LoaderCircle, Plus } from "lucide-react";
import type { Notify } from "../features/notifications/types";
import type { Skill } from "../features/skills/types";
import SkillCard from "./skills/SkillCard";
import SkillDeleteDialog from "./skills/SkillDeleteDialog";
import SkillsEmptyState from "./skills/SkillsEmptyState";

// Re-exported so the two skill surfaces keep one definition of the row. The
// composer picker and this screen describe the same table, and a shape that
// drifted between them would show as a skill missing a field in one place.
export type { Skill, SkillDraft } from "../features/skills/types";

/**
 * The skills wall: every skill as a card, grouped by type.
 *
 * **It does not edit.** Writing a skill happens on its own page, so this screen
 * only asks to go there -- `onOpenForm` names the skill or `null` for a new one,
 * and the settings shell switches the view. That keeps the form's draft state
 * from living beside the list's, which is what let the old modal drift out of
 * step with the cards behind it.
 *
 * **Grouped by `type` rather than listed flat**, because the label is the only
 * organising idea a skill has -- there is no folder tree -- and a user who
 * tagged three skills `frontend` wants to see those three together, under one
 * heading, without the label repeated on every card.
 *
 * **Disabled skills stay visible**, drawn on the quieter surface rather than
 * hidden. Hiding them would make "where did that skill go" answerable only by
 * turning a filter back on.
 */
export default function SkillsPage({
  notify,
  onOpenForm,
}: {
  notify: Notify;
  /** Opens the create/edit page: the skill to edit, or `null` for a new one. */
  onOpenForm: (skill: Skill | null) => void;
}) {
  const [skills, setSkills] = useState<Skill[]>([]);
  const [loading, setLoading] = useState(true);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);

  const reload = useCallback(async () => {
    try {
      setSkills(await invoke<Skill[]>("skills_list"));
    } catch (reason) {
      notify("error", `Skills could not be loaded: ${String(reason)}`);
    } finally {
      setLoading(false);
    }
  }, [notify]);

  useEffect(() => { void reload(); }, [reload]);

  const toggle = async (skill: Skill) => {
    try {
      await invoke("skills_set_enabled", { id: skill.id, enabled: !skill.enabled });
      await reload();
    } catch (reason) {
      notify("error", `Could not change this skill: ${String(reason)}`);
    }
  };

  const remove = async (skill: Skill) => {
    setConfirmDelete(null);
    try {
      await invoke("skills_delete", { id: skill.id });
      await reload();
      notify("success", `${skill.name} deleted`);
    } catch (reason) {
      notify("error", `Could not delete this skill: ${String(reason)}`);
    }
  };

  const grouped = groupByType(skills);

  return (
    <div>
      <div className="mb-6 flex flex-wrap items-start justify-between gap-3">
        <div>
          <h2 className="text-xl font-semibold tracking-tight">Skills</h2>
          <p className="mt-1 text-sm text-[var(--muted)]">
            Reusable instructions you or the agent can apply to a reply.
          </p>
        </div>
        <button
          type="button"
          onClick={() => onOpenForm(null)}
          className="inline-flex min-h-10 shrink-0 items-center gap-2 rounded-lg border border-[var(--line)] bg-[var(--panel)] px-3.5 text-sm font-medium text-[var(--text)] transition-colors hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
        >
          <Plus size={16} />New skill
        </button>
      </div>

      {loading ? (
        <p className="flex items-center gap-2 text-sm text-[var(--muted)]">
          <LoaderCircle size={14} className="animate-spin" />Loading skills…
        </p>
      ) : skills.length === 0 ? (
        <SkillsEmptyState onCreate={() => onOpenForm(null)} />
      ) : (
        <div className="space-y-7">
          {grouped.map(([label, group]) => (
            <section key={label}>
              <h3 className="mb-3 text-[11px] uppercase tracking-wide text-[var(--quiet)]">
                {label}
              </h3>
              <div className="grid gap-3 sm:grid-cols-2">
                {group.map((skill) => (
                  <SkillCard
                    key={skill.id}
                    skill={skill}
                    onEdit={onOpenForm}
                    onToggle={(target) => void toggle(target)}
                    onDelete={(target) => setConfirmDelete(target.id)}
                  />
                ))}
              </div>
            </section>
          ))}
        </div>
      )}

      {confirmDelete && (
        <SkillDeleteDialog
          name={skills.find((skill) => skill.id === confirmDelete)?.name ?? "this skill"}
          onConfirm={() => {
            const skill = skills.find((entry) => entry.id === confirmDelete);
            if (skill) void remove(skill);
          }}
          onCancel={() => setConfirmDelete(null)}
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
 * heading per card.
 */
function groupByType(skills: Skill[]): [string, Skill[]][] {
  const groups = new Map<string, Skill[]>();
  for (const skill of skills) {
    const label = skill.skill_type?.trim() || "Unlabelled";
    groups.set(label, [...(groups.get(label) ?? []), skill]);
  }
  return [...groups.entries()].sort(([left], [right]) =>
    // Unlabelled last: it is the absence of a choice, not a choice.
    left === "Unlabelled" ? 1 : right === "Unlabelled" ? -1 : left.localeCompare(right),
  );
}
