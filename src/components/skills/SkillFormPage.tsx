import { type FormEvent, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { LoaderCircle } from "lucide-react";
import type { Notify } from "../../features/notifications/types";
import {
  blankSkillDraft,
  toSkillPayload,
  type Skill,
  type SkillDraft,
} from "../../features/skills/types";

/**
 * The full-page create/edit form for one skill.
 *
 * **A page, not a dialog.** Writing instructions is the longest thing anyone
 * does with a skill, and the body is the part that wants room -- so the form
 * takes the whole settings surface the way the provider form does, and the list
 * stays one Back press away. A dialog put an 8,000-character textarea inside a
 * box too small to read it in.
 *
 * **It writes and reports, then hands back.** The command goes out from here and
 * the outcome is named in the notification stack; returning to the list is the
 * caller's job. The list re-reads on mount, so nothing has to tell it what
 * changed -- which is what keeps this page from having to know about the list.
 */
export default function SkillFormPage({
  skill,
  onSaved,
  onCancel,
  notify,
}: {
  /** The skill being edited, or `null` when writing a new one. */
  skill: Skill | null;
  onSaved: () => void;
  onCancel: () => void;
  notify: Notify;
}) {
  const [draft, setDraft] = useState<SkillDraft>(() =>
    skill
      ? {
          name: skill.name,
          description: skill.description ?? "",
          instructions: skill.instructions,
          skillType: skill.skill_type ?? "",
        }
      : blankSkillDraft(),
  );
  const [saving, setSaving] = useState(false);
  // This form's own validation, kept inline beside the fields because it points
  // at one of them by focusing it. A failure that comes back from the write is
  // shown here too, since the backend's message names the offending field.
  const [formError, setFormError] = useState("");
  const nameRef = useRef<HTMLInputElement>(null);
  const instructionsRef = useRef<HTMLTextAreaElement>(null);

  const field = (key: keyof SkillDraft) => (event: { target: { value: string } }) =>
    setDraft({ ...draft, [key]: event.target.value });

  const save = async (event: FormEvent) => {
    event.preventDefault();
    // Trimmed before checking, matching the backend: a name of only spaces is
    // empty there, and a form that accepted it would fail a moment later for a
    // reason the user cannot see.
    if (!draft.name.trim()) {
      setFormError("A skill needs a name");
      nameRef.current?.focus();
      return;
    }
    if (!draft.instructions.trim()) {
      setFormError("A skill needs instructions to apply");
      instructionsRef.current?.focus();
      return;
    }
    setSaving(true);
    try {
      setFormError("");
      if (skill) {
        // Sends only the editable fields. `origin`, `enabled` and `use_count`
        // are not in the payload at all, so an edit cannot reset them.
        await invoke("skills_update", { id: skill.id, skill: toSkillPayload(draft) });
      } else {
        await invoke("skills_create", { skill: toSkillPayload(draft) });
      }
      notify("success", skill ? `${draft.name} updated` : `${draft.name} created`);
      onSaved();
    } catch (reason) {
      setFormError(String(reason));
    } finally {
      setSaving(false);
    }
  };

  return (
    <form onSubmit={(event) => void save(event)} className="max-w-xl">
      {/* The shell header already names this page, so the title this used to
          carry was a title under a title. Kept as a heading for the document
          outline, kept out of the way of the eye. */}
      <h2 className="sr-only">{skill ? "Edit skill" : "New skill"}</h2>

      {/* Name and Type share a row: they are one thought -- what the skill is
          and what it files under -- and a short label does not need a row to
          itself. The name takes the wider share because a name is a phrase and
          a label is a word. */}
      <div className="grid gap-4 sm:grid-cols-[minmax(0,2fr)_minmax(0,1fr)]">
        <label className="block text-sm text-[var(--muted)]">
          Name
          <input
            ref={nameRef}
            required
            value={draft.name}
            onChange={field("name")}
            maxLength={80}
            placeholder="Rust house style"
            className="mt-1.5 h-10 w-full rounded-md border border-[var(--line)] bg-[var(--rail)] px-3 text-sm text-[var(--text)] outline-none placeholder:text-[var(--quiet)] focus:border-[var(--accent)]"
          />
        </label>

        <label className="block text-sm text-[var(--muted)]">
          Type <span className="text-[var(--quiet)]">(optional)</span>
          <input
            value={draft.skillType}
            onChange={field("skillType")}
            placeholder="frontend"
            className="mt-1.5 h-10 w-full rounded-md border border-[var(--line)] bg-[var(--rail)] px-3 text-sm text-[var(--text)] outline-none placeholder:text-[var(--quiet)] focus:border-[var(--accent)]"
          />
        </label>

        <label className="block text-sm text-[var(--muted)] sm:col-span-2">
          Description <span className="text-[var(--quiet)]">(optional)</span>
          <input
            value={draft.description}
            onChange={field("description")}
            maxLength={300}
            placeholder="How we write Rust in this project"
            className="mt-1.5 h-10 w-full rounded-md border border-[var(--line)] bg-[var(--rail)] px-3 text-sm text-[var(--text)] outline-none placeholder:text-[var(--quiet)] focus:border-[var(--accent)]"
          />
          <span className="mt-1 block text-xs text-[var(--quiet)]">One line for the card.</span>
        </label>

        {/* The substance of a skill, so it gets the room: the tallest field on
            the page, and the one the eye lands on after the name. */}
        <label className="block text-sm text-[var(--muted)] sm:col-span-2">
          Instructions
          <textarea
            ref={instructionsRef}
            required
            value={draft.instructions}
            onChange={field("instructions")}
            rows={12}
            maxLength={8000}
            placeholder={"- Prefer returning Result over panicking\n- Keep public APIs small\n- Comment the why, not the what"}
            className="mt-1.5 min-h-56 w-full resize-y rounded-md border border-[var(--line)] bg-[var(--rail)] px-3 py-2.5 text-sm leading-6 text-[var(--text)] outline-none placeholder:text-[var(--quiet)] focus:border-[var(--accent)]"
          />
          <span className="mt-1 block text-xs text-[var(--quiet)]">What the model should do when this applies.</span>
        </label>
      </div>

      {formError && <p role="alert" className="mt-4 text-sm text-[var(--danger)]">{formError}</p>}

      <div className="mt-6 flex flex-wrap gap-2 border-t border-[var(--line)] pt-4">
        <button
          type="submit"
          disabled={saving}
          className="inline-flex min-h-10 items-center gap-2 rounded-md bg-[var(--accent)] px-4 text-sm font-semibold text-[var(--accent-ink)] hover:opacity-90 disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
        >
          {saving && <LoaderCircle size={15} className="animate-spin" />}
          {skill ? "Save changes" : "Create skill"}
        </button>
        <button
          type="button"
          onClick={onCancel}
          className="min-h-10 rounded-md px-3 text-sm text-[var(--muted)] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
        >
          Cancel
        </button>
      </div>
    </form>
  );
}
