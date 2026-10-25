import { FileText, Folder, Pencil, SquareTerminal, Trash2 } from "lucide-react";
import type { Skill } from "../../features/skills/types";

/**
 * One skill as a row.
 *
 * **A row, not a card.** A skill is one line of text and a folder; a grid of
 * cards spent four times the height to say the same thing and buried the two
 * controls under a family heading that no longer exists. Reading down the title
 * column is how a person finds the one they mean.
 *
 * **The glyphs are the folder.** `scripts/`, `references/` and `assets/` are the
 * three things a skill can carry beside its body, and each is drawn only when it
 * is there -- a row with no files says nothing rather than listing zeros.
 *
 * **Double click opens the folder.** One click selects nothing and edits nothing;
 * two open the skill's folder in the Files panel, which is where it is actually
 * edited. The edit button exists because a double click is not discoverable on
 * its own, and it does the same thing.
 */
export default function SkillRow({
  skill,
  index,
  onOpen,
  onDelete,
}: {
  skill: Skill;
  /** Position in the list, so the alternating surface can be a pure CSS rule. */
  index: number;
  /** Opens the skill's folder in the right panel. */
  onOpen: (skill: Skill) => void;
  onDelete: (skill: Skill) => void;
}) {
  return (
    <div
      onDoubleClick={() => onOpen(skill)}
      title="Double-click to open the folder"
      data-skill-row
      className={`group flex min-h-11 items-center gap-3 px-3 py-2 transition-colors hover:bg-[var(--raised)] ${
        index % 2 === 1 ? "bg-[color-mix(in_srgb,var(--rail)_55%,transparent)]" : ""
      }`}
    >
      <span className="w-[30%] min-w-0 shrink-0 truncate text-[13px] font-medium text-[var(--text)]" title={skill.slug}>
        {skill.name}
      </span>

      <span className="min-w-0 flex-1 truncate text-[12px] text-[var(--muted)]" title={skill.description ?? undefined}>
        {skill.description ?? (
          <span className="text-[var(--quiet)]">No description — the agent has no trigger to match on.</span>
        )}
      </span>

      <span className="flex shrink-0 items-center gap-1 text-[var(--quiet)]">
        {skill.has_scripts && <Glyph icon={SquareTerminal} label="scripts" />}
        {skill.has_references && <Glyph icon={FileText} label="references" />}
        {skill.has_assets && <Glyph icon={Folder} label="assets" />}
      </span>

      {/* Always drawn, never revealed on hover: a control that appears under the
          pointer is one nobody finds by scanning the list, and the row is a
          stable height either way. */}
      <span className="flex shrink-0 items-center gap-0.5">
        <button
          type="button"
          onClick={() => onOpen(skill)}
          aria-label={`Edit ${skill.name}`}
          title="Edit files"
          className="grid size-7 place-items-center rounded-md text-[var(--quiet)] transition-colors hover:bg-[var(--panel)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
        >
          <Pencil size={14} aria-hidden />
        </button>
        <button
          type="button"
          onClick={() => onDelete(skill)}
          aria-label={`Delete ${skill.name}`}
          title="Delete"
          className="grid size-7 place-items-center rounded-md text-[var(--quiet)] transition-colors hover:bg-[var(--panel)] hover:text-[var(--danger)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
        >
          <Trash2 size={14} aria-hidden />
        </button>
      </span>
    </div>
  );
}

/** One of the three folder kinds, named on hover and in the accessibility tree. */
function Glyph({ icon: Icon, label }: { icon: typeof FileText; label: string }) {
  return (
    <span className="grid size-6 place-items-center" title={label} aria-label={label}>
      <Icon size={13} aria-hidden />
    </span>
  );
}

