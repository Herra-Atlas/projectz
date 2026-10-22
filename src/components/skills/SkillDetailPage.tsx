import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openPath } from "@tauri-apps/plugin-opener";
import {
  FolderOpen,
  LoaderCircle,
  Save,
  ShieldAlert,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import { HoldToDelete } from "../HoldToDelete";
import { SettingsPageTitle, Toggle } from "../SettingsSection";
import SkillFilesPanel from "./SkillFilesPanel";
import {
  blankSkillDraft,
  toSkillPayload,
  type Skill,
  type SkillDetail,
  type SkillDraft,
} from "../../features/skills/types";
import type { Notify } from "../../features/notifications/types";

/**
 * The create/edit page for one skill.
 *
 * **A page, not a dialog.** Writing SKILL.md is the longest thing anyone does
 * here, and the body wants room. The list stays one Back press away.
 *
 * **The screen is the folder.** The left column is the skill's own text
 * (`SKILL.md`); the right column is everything around it -- the two switches,
 * the files, and the one destructive action. That split mirrors the storage, so
 * a user who has opened the folder in Explorer recognises the page: what is in
 * the file is on the left, what is beside it is on the right.
 *
 * **Saving is explicit.** Nothing is autosaved. `SKILL.md` is the contract with
 * the model, and a half-typed trigger line changing what the agent reads is not a
 * background event. The file editor inside the files panel saves its own file,
 * because that is a different unit of work.
 */
export default function SkillDetailPage({
  slug,
  notify,
  onSaved,
  onCancel,
  onDeleted,
}: {
  /** The skill being edited, or `null` for a new one. */
  slug: string | null;
  notify: Notify;
  /** Called with the slug after a save; the shell only navigates on create. */
  onSaved: (slug: string) => void;
  onCancel: () => void;
  onDeleted: () => void;
}) {
  const [detail, setDetail] = useState<SkillDetail | null>(null);
  const [draft, setDraft] = useState<SkillDraft>(blankSkillDraft);
  const [loading, setLoading] = useState(Boolean(slug));
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const nameRef = useRef<HTMLInputElement>(null);
  const bodyRef = useRef<HTMLTextAreaElement>(null);

  const load = useCallback(
    async (target: string) => {
      try {
        const next = await invoke<SkillDetail>("skills_get", { slug: target });
        setDetail(next);
        setDraft({
          name: next.skill.name,
          description: next.skill.description ?? "",
          instructions: next.instructions,
          skillType: next.skill.skill_type ?? "",
        });
        setError("");
      } catch (reason) {
        setError(String(reason));
      } finally {
        setLoading(false);
      }
    },
    [],
  );

  useEffect(() => {
    if (slug) void load(slug);
  }, [slug, load]);

  /**
   * Re-reads the skill's state and file list after a file changes.
   *
   * **Not `load`.** This is called while the body may have unsaved edits, and
   * `load` also resets the draft from disk -- adding a reference file would
   * silently throw away the paragraph the user was halfway through. Only the
   * parts the file change can affect are replaced.
   */
  const refresh = useCallback(async (target: string) => {
    try {
      setDetail(await invoke<SkillDetail>("skills_get", { slug: target }));
    } catch (reason) {
      setError(String(reason));
    }
  }, []);

  const field = (key: keyof SkillDraft) => (event: { target: { value: string } }) =>
    setDraft((current) => ({ ...current, [key]: event.target.value }));

  const save = async () => {
    if (!draft.name.trim()) {
      setError("A skill needs a name.");
      nameRef.current?.focus();
      return;
    }
    if (!draft.instructions.trim()) {
      setError("A skill needs instructions to apply.");
      bodyRef.current?.focus();
      return;
    }
    setSaving(true);
    setError("");
    try {
      const skill = slug
        ? await invoke<Skill>("skills_update", {
            slug,
            skill: toSkillPayload(draft),
          })
        : await invoke<Skill>("skills_create", { skill: toSkillPayload(draft) });
      notify("success", slug ? `${skill.name} saved` : `${skill.name} created`);
      onSaved(skill.slug);
      if (slug) await load(slug);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setSaving(false);
    }
  };

  /** A switch records itself and then reports; a failure puts it back. */
  const setSwitch = async (
    kind: "enabled" | "trusted",
    next: boolean,
  ) => {
    if (!detail) return;
    const previous = detail.skill[kind];
    setDetail({ ...detail, skill: { ...detail.skill, [kind]: next } });
    try {
      await invoke(kind === "enabled" ? "skills_set_enabled" : "skills_set_trusted", {
        slug: detail.skill.slug,
        [kind]: next,
      });
    } catch (reason) {
      setDetail((current) =>
        current ? { ...current, skill: { ...current.skill, [kind]: previous } } : current,
      );
      notify("error", String(reason));
    }
  };

  const reveal = async () => {
    if (!detail) return;
    try {
      await openPath(await invoke<string>("skills_folder", { slug: detail.skill.slug }));
    } catch (reason) {
      notify("error", `Could not open the folder: ${String(reason)}`);
    }
  };

  const remove = async () => {
    if (!slug) return;
    try {
      await invoke("skills_delete", { slug });
      notify("success", `${detail?.skill.name ?? "Skill"} deleted`);
      onDeleted();
    } catch (reason) {
      notify("error", String(reason));
    }
  };

  if (loading) {
    return (
      <p className="flex items-center gap-2 text-sm text-[var(--muted)]">
        <LoaderCircle size={14} className="animate-spin" /> Reading the skill…
      </p>
    );
  }

  const skill = detail?.skill;
  const isNew = !slug;

  return (
    <div>
      <SettingsPageTitle>
        <div className="min-w-0">
          <h2 className="truncate text-[17px] font-semibold tracking-tight">
            {isNew ? "New skill" : skill?.name ?? "Skill"}
          </h2>
          <p className="mt-0.5 truncate text-[11.5px] text-[var(--quiet)]">
            {isNew
              ? "A folder with SKILL.md, and any scripts or references it needs."
              : `skills/${skill?.slug} · used ${skill?.use_count ?? 0}× · ${originLabel(skill?.origin)}`}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {!isNew && (
            <button
              type="button"
              onClick={() => void reveal()}
              className="inline-flex min-h-9 items-center gap-1.5 rounded-lg border border-[var(--line)] bg-[var(--panel)] px-3 text-[12.5px] font-medium text-[var(--muted)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
            >
              <FolderOpen size={14} /> Show folder
            </button>
          )}
          <button
            type="button"
            onClick={onCancel}
            className="min-h-9 rounded-lg px-3 text-[12.5px] text-[var(--muted)] transition-colors hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
          >
            {isNew ? "Cancel" : "Back"}
          </button>
          <button
            type="button"
            onClick={() => void save()}
            disabled={saving}
            className="inline-flex min-h-9 items-center gap-1.5 rounded-lg bg-[var(--accent)] px-3.5 text-[12.5px] font-semibold text-[var(--accent-ink)] transition-opacity hover:opacity-90 disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
          >
            {saving ? <LoaderCircle size={14} className="animate-spin" /> : <Save size={14} />}
            {isNew ? "Create skill" : "Save"}
          </button>
        </div>
      </SettingsPageTitle>

      <div className="grid gap-8 lg:grid-cols-[minmax(0,1fr)_300px]">
        <div className="min-w-0 space-y-5">
          <div className="grid gap-4 sm:grid-cols-[minmax(0,2fr)_minmax(0,1fr)]">
            <label className="block text-[12.5px] text-[var(--muted)]">
              Name
              <input
                ref={nameRef}
                value={draft.name}
                onChange={field("name")}
                maxLength={120}
                placeholder="Code review"
                className="mt-1.5 h-9 w-full rounded-lg border border-[var(--line)] bg-[var(--rail)] px-3 text-[13px] text-[var(--text)] outline-none placeholder:text-[var(--quiet)] focus:border-[var(--accent)]"
              />
              {!isNew && (
                <span className="mt-1 block text-[10.5px] text-[var(--quiet)]">
                  The folder keeps its address when this changes.
                </span>
              )}
            </label>

            <label className="block text-[12.5px] text-[var(--muted)]">
              Type <span className="text-[var(--quiet)]">(optional)</span>
              <input
                value={draft.skillType}
                onChange={field("skillType")}
                placeholder="code"
                className="mt-1.5 h-9 w-full rounded-lg border border-[var(--line)] bg-[var(--rail)] px-3 text-[13px] text-[var(--text)] outline-none placeholder:text-[var(--quiet)] focus:border-[var(--accent)]"
              />
            </label>
          </div>

          <label className="block text-[12.5px] text-[var(--muted)]">
            Description
            <input
              value={draft.description}
              onChange={field("description")}
              placeholder="Use when reviewing a diff before it ships"
              className="mt-1.5 h-9 w-full rounded-lg border border-[var(--line)] bg-[var(--rail)] px-3 text-[13px] text-[var(--text)] outline-none placeholder:text-[var(--quiet)] focus:border-[var(--accent)]"
            />
            <span className="mt-1 block text-[10.5px] text-[var(--quiet)]">
              The only text the agent sees before deciding to read this. Write it as a trigger.
            </span>
          </label>

          <label className="block text-[12.5px] text-[var(--muted)]">
            Instructions
            <textarea
              ref={bodyRef}
              value={draft.instructions}
              onChange={field("instructions")}
              rows={18}
              spellCheck={false}
              placeholder={"1. Read the diff and the files it touches.\n2. Check the tests cover the change.\n3. Report blockers first."}
              className="mt-1.5 w-full resize-y rounded-lg border border-[var(--line)] bg-[var(--rail)] px-3 py-2.5 font-mono text-[12px] leading-6 text-[var(--text)] outline-none placeholder:text-[var(--quiet)] focus:border-[var(--accent)]"
            />
            <span className="mt-1 flex items-center justify-between text-[10.5px] text-[var(--quiet)]">
              <span>Markdown. The agent reads this when the description matches.</span>
              <span>{draft.instructions.length.toLocaleString()} chars</span>
            </span>
          </label>
        </div>

        <aside className="space-y-4">
          {skill && (
            <section className="rounded-xl border border-[var(--line)] bg-[var(--panel)]">
              <header className="border-b border-[var(--line)] px-4 py-3">
                <h3 className="text-[12.5px] font-medium text-[var(--text)]">Status</h3>
              </header>
              <div className="divide-y divide-[var(--line)]">
                <div className="flex items-start justify-between gap-3 px-4 py-3">
                  <div className="min-w-0">
                    <p className="text-[12px] text-[var(--text)]">Offered to the agent</p>
                    <p className="mt-0.5 text-[10.5px] leading-4 text-[var(--quiet)]">
                      Listed in the prompt so it can be chosen.
                    </p>
                  </div>
                  <Toggle
                    checked={skill.enabled}
                    onChange={(next) => void setSwitch("enabled", next)}
                    label="Offer this skill"
                  />
                </div>

                <div className="px-4 py-3">
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0">
                      <p className="flex items-center gap-1.5 text-[12px] text-[var(--text)]">
                        {skill.trusted ? (
                          <ShieldCheck size={13} className="text-[var(--accent)]" aria-hidden />
                        ) : (
                          <ShieldAlert size={13} className="text-[var(--quiet)]" aria-hidden />
                        )}
                        Allow scripts
                      </p>
                      <p className="mt-0.5 text-[10.5px] leading-4 text-[var(--quiet)]">
                        {skill.has_scripts
                          ? "Lets the agent run files in scripts/. They run with your permissions."
                          : "No scripts yet. This switch only matters once there are some."}
                      </p>
                    </div>
                    <Toggle
                      checked={skill.trusted}
                      onChange={(next) => void setSwitch("trusted", next)}
                      label="Allow this skill's scripts"
                    />
                  </div>
                </div>
              </div>
            </section>
          )}

          {skill ? (
            <SkillFilesPanel
              slug={skill.slug}
              files={detail?.files ?? []}
              onChanged={() => refresh(skill.slug)}
              notify={notify}
            />
          ) : (
            <section className="rounded-xl border border-dashed border-[var(--line)] px-4 py-4">
              <p className="text-[11.5px] leading-5 text-[var(--quiet)]">
                Create the skill first, then add scripts and references here.
              </p>
            </section>
          )}

          {skill && (
            <section className="rounded-xl border border-[var(--line)] bg-[var(--panel)] p-3">
              <HoldToDelete
                onConfirm={() => void remove()}
                label={`Delete ${skill.name}`}
                title="Hold to delete this skill and its folder"
                className="flex w-full items-center gap-2 rounded-lg px-2.5 py-2 text-[11.5px] text-[var(--quiet)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--danger)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
              >
                <Trash2 size={13} />
                Hold to delete this skill
              </HoldToDelete>
            </section>
          )}
        </aside>
      </div>

      {error && (
        <p role="alert" className="mt-5 rounded-lg border border-[var(--line)] bg-[var(--panel)] px-3 py-2 text-[11.5px] text-[var(--danger)]">
          {error}
        </p>
      )}
    </div>
  );
}

function originLabel(origin: Skill["origin"] | undefined): string {
  if (origin === "bundled") return "included";
  if (origin === "generated") return "drafted by the agent";
  return "yours";
}
