import { Check, Sparkles, Trash2 } from "lucide-react";
import type { Skill } from "../../features/skills/types";

/**
 * One skill drawn as a card.
 *
 * **The body is the edit affordance.** The card's name, description and meta
 * live inside a single button, so the whole area a reader points at opens the
 * editor -- there is no separate pencil to find. The two controls that are not
 * "open the editor", enable and delete, sit in their own bar below the body:
 * a button cannot contain a button, and a click meant to disable a skill should
 * not also be read as a click meant to edit it.
 *
 * **Enabled is carried by the surface, not by opacity.** An off skill is drawn
 * on the rail colour and its text steps back to muted; an on skill is drawn on
 * the panel colour, one step lighter. On a dark theme lightness is the readable
 * elevation cue, and an opacity veil would dim the controls along with the words
 * instead of just settling the card.
 *
 * **Nothing here is hover-only.** Delete is quiet but always present, because a
 * control that only exists under a pointer is a control a keyboard never finds.
 */
export default function SkillCard({
  skill,
  onEdit,
  onToggle,
  onDelete,
}: {
  skill: Skill;
  onEdit: (skill: Skill) => void;
  onToggle: (skill: Skill) => void;
  onDelete: (skill: Skill) => void;
}) {
  const on = skill.enabled;
  return (
    <article
      className={`group flex flex-col overflow-hidden rounded-xl border border-[var(--line)] transition-[border-color,box-shadow] duration-150 hover:border-[color-mix(in_srgb,var(--accent)_40%,var(--line))] hover:shadow-[0_8px_24px_rgba(0,0,0,0.22)] focus-within:border-[color-mix(in_srgb,var(--accent)_45%,var(--line))] ${
        on ? "bg-[var(--panel)]" : "bg-[var(--rail)]"
      }`}
    >
      <button
        type="button"
        onClick={() => onEdit(skill)}
        aria-label={`Edit ${skill.name}`}
        className="flex flex-1 flex-col gap-2.5 p-4 text-left focus-visible:outline-2 focus-visible:outline-offset-[-3px] focus-visible:outline-[var(--accent)]"
      >
        <span className={`block truncate text-sm font-medium ${on ? "text-[var(--text)]" : "text-[var(--muted)]"}`}>
          {skill.name}
        </span>
        {/* A fixed two-line box, so cards in a row agree on where their meta
            sits whether a skill carries no description or a long one. */}
        <span className={`line-clamp-2 block min-h-10 text-xs leading-5 ${on ? "text-[var(--muted)]" : "text-[var(--quiet)]"}`}>
          {skill.description || "No description."}
        </span>
        <span className="mt-auto flex items-center gap-2 text-[10px] text-[var(--quiet)]">
          {skill.origin === "generated" && (
            // Named because an agent-drafted skill encodes an assumption nobody
            // checked, and this is the list someone re-reads to find them.
            <span className="inline-flex items-center gap-1 rounded bg-[var(--raised)] px-1.5 py-0.5">
              <Sparkles size={11} aria-hidden /> Drafted by agent
            </span>
          )}
          {skill.use_count > 0 && <span>used {skill.use_count}×</span>}
        </span>
      </button>

      <div className="flex items-center justify-between gap-2 border-t border-[var(--line)] px-3 py-2">
        <button
          type="button"
          role="switch"
          aria-checked={on}
          aria-label={skill.name}
          onClick={() => onToggle(skill)}
          className="inline-flex min-h-8 items-center gap-2 rounded-md px-1.5 text-xs text-[var(--muted)] transition-colors hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
        >
          <span
            className={`grid size-4 shrink-0 place-items-center rounded border transition-colors ${
              on
                ? "border-[var(--accent)] bg-[var(--accent)] text-[var(--accent-ink)]"
                : "border-[var(--line)] text-transparent"
            }`}
          >
            <Check size={11} strokeWidth={3} />
          </span>
          {on ? "Enabled" : "Off"}
        </button>

        <button
          type="button"
          onClick={() => onDelete(skill)}
          aria-label={`Delete ${skill.name}`}
          className="grid size-8 shrink-0 place-items-center rounded-md text-[var(--quiet)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--danger)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
        >
          <Trash2 size={14} />
        </button>
      </div>
    </article>
  );
}
