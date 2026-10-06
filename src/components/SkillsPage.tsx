import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Check, LoaderCircle, Pencil, Plus, Sparkles, Trash2, X } from "lucide-react";
import type { Notify } from "../features/notifications/types";
import {
  blankSkillDraft,
  toSkillPayload,
  type Skill,
  type SkillDraft,
} from "../features/skills/types";

// Re-exported so the two skill surfaces keep one definition of the row. The
// composer picker and this screen describe the same table, and a shape that
// drifted between them would show as a skill missing a field in one place.
export type { Skill, SkillDraft };

/**
 * Create, edit, enable and delete skills.
 *
 * Grouped by `type` rather than listed flat, because the label is the only
 * organising idea a skill has -- there is no folder tree -- and a user who tagged
 * three skills `frontend` wants to see those three together.
 *
 * Disabled skills stay visible. Hiding them would make "where did that skill
 * go" answerable only by turning a filter back on.
 */
export default function SkillsPage({ notify }: { notify: Notify }) {
  const [skills, setSkills] = useState<Skill[]>([]);
  const [loading, setLoading] = useState(true);
  // Failures go to the notification stack rather than to a line on this page. A
  // skill that could not be saved has to be said somewhere the user is looking
  // after they have clicked Save and the editor has closed -- an inline line
  // above a list that has just re-rendered is the first thing to be missed.
  const [editing, setEditing] = useState<Skill | "new" | null>(null);
  const [draft, setDraft] = useState<SkillDraft>(blankSkillDraft());
  const [saving, setSaving] = useState(false);
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

  const startNew = () => { setDraft(blankSkillDraft()); setEditing("new"); };
  const startEdit = (skill: Skill) => {
    setDraft({
      name: skill.name,
      description: skill.description ?? "",
      instructions: skill.instructions,
      skillType: skill.skill_type ?? "",
    });
    setEditing(skill);
  };

  const save = async () => {
    setSaving(true);
    try {
      if (editing === "new") {
        await invoke("skills_create", { skill: toSkillPayload(draft) });
      } else if (editing) {
        // Sends only the editable fields. `origin`, `enabled` and `use_count`
        // are not in the payload at all, so an edit cannot reset them.
        await invoke("skills_update", { id: editing.id, skill: toSkillPayload(draft) });
      }
      const saved = editing;
      setEditing(null);
      await reload();
      // Confirmed after the reload rather than before, so the message names the
      // skill that is now actually on screen and not one that failed to save.
      if (saved === "new") notify("success", `${draft.name} created`);
      else if (saved) notify("success", `${draft.name} updated`);
    } catch (reason) {
      // The backend's validation messages name the field that is wrong, so they
      // are shown as-is rather than replaced with something vaguer.
      notify("error", String(reason));
    } finally {
      setSaving(false);
    }
  };

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
      <div className="mb-6 flex items-start justify-between gap-4">
        <div>
          <h2 className="text-xl font-semibold tracking-tight">Skills</h2>
          <p className="mt-1 text-sm text-[var(--muted)]">
            Reusable instructions you or the agent can apply to a reply.
          </p>
        </div>
        <button
          type="button"
          onClick={startNew}
          className="inline-flex min-h-9 shrink-0 items-center gap-1.5 rounded-md border border-[var(--line)] px-3 text-xs text-[var(--text)] transition-colors hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
        >
          <Plus size={14} />New skill
        </button>
      </div>

      {loading ? (
        <p className="flex items-center gap-2 text-sm text-[var(--muted)]">
          <LoaderCircle size={14} className="animate-spin" />Loading skills…
        </p>
      ) : skills.length === 0 && !editing ? (
        <EmptyState onCreate={startNew} />
      ) : (
        <div className="space-y-6">
          {grouped.map(([label, group]) => (
            <section key={label}>
              <h3 className="mb-2 text-[11px] uppercase tracking-wide text-[var(--quiet)]">
                {label}
              </h3>
              <ul className="overflow-hidden rounded-lg border border-[var(--line)]">
                {group.map((skill) => (
                  <li
                    key={skill.id}
                    className="flex items-start gap-3 border-b border-[var(--line)] px-3 py-2.5 last:border-b-0"
                  >
                    <button
                      type="button"
                      onClick={() => toggle(skill)}
                      role="switch"
                      aria-checked={skill.enabled}
                      aria-label={skill.enabled ? `Disable ${skill.name}` : `Enable ${skill.name}`}
                      className={`mt-0.5 grid size-5 shrink-0 place-items-center rounded border focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)] ${
                        skill.enabled
                          ? "border-[var(--accent)] bg-[var(--accent)] text-[var(--panel)]"
                          : "border-[var(--line)] text-transparent"
                      }`}
                    >
                      <Check size={12} />
                    </button>

                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-2">
                        <span className={`truncate text-sm ${skill.enabled ? "text-[var(--text)]" : "text-[var(--quiet)]"}`}>
                          {skill.name}
                        </span>
                        {skill.origin === "generated" && (
                          // Distinguished because an agent-drafted skill encodes an
                          // assumption that was never checked, and a user should
                          // be able to see which ones to re-read.
                          <span className="shrink-0 rounded bg-[var(--raised)] px-1.5 py-0.5 text-[10px] text-[var(--quiet)]">
                            Drafted by agent
                          </span>
                        )}
                        {skill.use_count > 0 && (
                          <span className="shrink-0 text-[10px] text-[var(--quiet)]">
                            used {skill.use_count}×
                          </span>
                        )}
                      </div>
                      {skill.description && (
                        <p className="mt-0.5 text-xs leading-5 text-[var(--muted)]">{skill.description}</p>
                      )}
                    </div>

                    <div className="flex shrink-0 gap-1">
                      <button
                        type="button"
                        onClick={() => startEdit(skill)}
                        aria-label={`Edit ${skill.name}`}
                        className="grid size-8 place-items-center rounded-md text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
                      >
                        <Pencil size={14} />
                      </button>
                      <button
                        type="button"
                        onClick={() => setConfirmDelete(skill.id)}
                        aria-label={`Delete ${skill.name}`}
                        className="grid size-8 place-items-center rounded-md text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--danger)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
                      >
                        <Trash2 size={14} />
                      </button>
                    </div>
                  </li>
                ))}
              </ul>
            </section>
          ))}
        </div>
      )}

      {editing && (
        <SkillEditor
          draft={draft}
          setDraft={setDraft}
          saving={saving}
          onSave={save}
          onCancel={() => setEditing(null)}
        />
      )}

      {confirmDelete && (
        <ConfirmDelete
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
 * heading per row.
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

function EmptyState({ onCreate }: { onCreate: () => void }) {
  return (
    <div className="rounded-lg border border-dashed border-[var(--line)] px-6 py-10">
      <div className="flex flex-col items-center">
        <Sparkles size={20} className="mb-2 text-[var(--quiet)]" />
        <p className="text-center text-sm text-[var(--text)]">No skills yet</p>
        <p className="mt-1 max-w-sm text-center text-xs leading-5 text-[var(--quiet)]">
          A skill is a set of instructions kept for reuse — how you want code written, what to
          check before calling something done, what to avoid.
        </p>
<button
          type="button"
          onClick={onCreate}
          className="mt-4 inline-flex min-h-9 items-center gap-1.5 rounded-md border border-[var(--line)] px-3 text-xs text-[var(--text)] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
        >
          <Plus size={14} />Write your first
        </button>
      </div>
    </div>
  );
}

function SkillEditor({
  draft,
  setDraft,
  saving,
  onSave,
  onCancel,
}: {
  draft: SkillDraft;
  setDraft: (draft: SkillDraft) => void;
  saving: boolean;
  onSave: () => void;
  onCancel: () => void;
}) {
  const field = (key: keyof SkillDraft) => (event: { target: { value: string } }) =>
    setDraft({ ...draft, [key]: event.target.value });

  return (
    <div className="fixed inset-0 z-50 grid place-items-center bg-black/60 p-4">
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="skill-editor-title"
        className="flex max-h-[85vh] w-full max-w-lg flex-col overflow-hidden rounded-lg border border-[var(--line)] bg-[var(--panel)] shadow-2xl"
      >
        <div className="flex items-center justify-between border-b border-[var(--line)] px-4 py-3">
          <h3 id="skill-editor-title" className="text-sm font-medium text-[var(--text)]">
            Skill
          </h3>
          <button
            type="button"
            onClick={onCancel}
            aria-label="Close"
            className="grid size-8 place-items-center rounded-md text-[var(--muted)] hover:bg-[var(--raised)]"
          >
            <X size={16} />
          </button>
        </div>

        <div className="min-h-0 flex-1 space-y-3 overflow-y-auto px-4 py-4">
          <Field label="Name">
            <input
              value={draft.name}
              onChange={field("name")}
              placeholder="Rust house style"
              maxLength={80}
              className="min-h-9 w-full rounded-md border border-[var(--line)] bg-[var(--rail)] px-2.5 text-sm text-[var(--text)] placeholder:text-[var(--quiet)] focus-visible:outline-2 focus-visible:outline-offset-[-1px] focus-visible:outline-[var(--accent)]"
            />
          </Field>

          <Field label="Description" hint="Optional. One line for the list.">
            <input
              value={draft.description}
              onChange={field("description")}
              placeholder="How we write Rust in this project"
              maxLength={300}
              className="min-h-9 w-full rounded-md border border-[var(--line)] bg-[var(--rail)] px-2.5 text-sm text-[var(--text)] placeholder:text-[var(--quiet)] focus-visible:outline-2 focus-visible:outline-offset-[-1px] focus-visible:outline-[var(--accent)]"
            />
          </Field>

          <Field label="Type" hint="Optional. Groups the list; any label works.">
            <input
              value={draft.skillType}
              onChange={field("skillType")}
              placeholder="frontend"
              className="min-h-9 w-full rounded-md border border-[var(--line)] bg-[var(--rail)] px-2.5 text-sm text-[var(--text)] placeholder:text-[var(--quiet)] focus-visible:outline-2 focus-visible:outline-offset-[-1px] focus-visible:outline-[var(--accent)]"
            />
          </Field>

          <Field label="Instructions" hint="What the model should do when this applies.">
            <textarea
              value={draft.instructions}
              onChange={field("instructions")}
              rows={8}
              maxLength={8000}
              placeholder={"- Prefer returning Result over panicking\n- Keep public APIs small\n- Comment the why, not the what"}
              className="w-full resize-y rounded-md border border-[var(--line)] bg-[var(--rail)] px-2.5 py-2 text-sm leading-6 text-[var(--text)] placeholder:text-[var(--quiet)] focus-visible:outline-2 focus-visible:outline-offset-[-1px] focus-visible:outline-[var(--accent)]"
            />
          </Field>
        </div>

        <div className="flex justify-end gap-2 border-t border-[var(--line)] px-4 py-3">
          <button
            type="button"
            onClick={onCancel}
            className="min-h-9 rounded-md px-3 text-xs text-[var(--muted)] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={onSave}
            disabled={saving}
            className="inline-flex min-h-9 items-center gap-1.5 rounded-md bg-[var(--accent)] px-3 text-xs font-medium text-[var(--panel)] hover:opacity-90 disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
          >
            {saving && <LoaderCircle size={13} className="animate-spin" />}
            Save
          </button>
        </div>
      </div>
    </div>
  );
}

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <label className="block">
      <span className="mb-1 block text-xs text-[var(--text)]">{label}</span>
      {children}
      {hint && <span className="mt-1 block text-[11px] leading-4 text-[var(--quiet)]">{hint}</span>}
    </label>
  );
}

/**
 * Asks before deleting, because this one is not undoable.
 *
 * The skills list has no undo and a skill is something a user wrote by hand, so
 * a single stray click would lose real work. Disabling is always available as the
 * reversible option.
 */
function ConfirmDelete({
  name,
  onConfirm,
  onCancel,
}: {
  name: string;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  return (
    <div className="fixed inset-0 z-50 grid place-items-center bg-black/60 p-4">
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="skill-delete-title"
        className="w-full max-w-sm rounded-lg border border-[var(--line)] bg-[var(--panel)] p-5 shadow-2xl"
        onKeyDown={(event) => { if (event.key === "Escape") onCancel(); }}
      >
        <h3 id="skill-delete-title" className="text-sm font-medium text-[var(--text)]">
          Delete this skill?
        </h3>
        <p className="mt-1.5 text-xs leading-5 text-[var(--muted)]">
          <span className="text-[var(--text)]">{name}</span> will be removed. This cannot be
          undone — disabling it keeps it instead.
        </p>
        <div className="mt-4 flex justify-end gap-2">
          <button
            type="button"
            onClick={onCancel}
            className="min-h-9 rounded-md px-3 text-xs text-[var(--muted)] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={onConfirm}
            className="min-h-9 rounded-md border border-[var(--danger)] px-3 text-xs text-[var(--danger)] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
          >
            Delete
          </button>
        </div>
      </div>
    </div>
  );
}