import { Check, Sparkles } from "lucide-react";
import type { Skill } from "../../features/skills/types";

/**
 * The skills submenu, opened by hovering the composer's Skills row.
 *
 * Presentational on purpose. It holds no selection of its own -- `selected` and
 * `onToggle` come from the page, which is where the send lives, because a skill
 * applies to a message and the message is the page's business.
 *
 * **A skill is a checkbox, not a jump.** There is nowhere to go, because there is
 * nothing to preview: the instructions are the whole of what a skill is, and
 * showing them would mean putting an 8,000-character body in a 260px menu.
 *
 * **An applied skill is shown checked and cannot be unchecked.** It is in the
 * conversation's transcript, so it is governing every turn that follows and
 * removing it would mean editing history. Saying so is better than a row that
 * appears to work.
 *
 * **The panel starts at the row's own edge.** `left-full ml-1` left a 4px gap the
 * pointer had to cross, and crossing it fired the row's `mouseleave` -- so the
 * panel closed before it could be reached. The gap is now inside the panel, as
 * transparent padding, so the hover region is unbroken.
 */
export default function SkillsSubmenu({
  skills,
  selected,
  applied,
  loading,
  onToggle,
}: {
  skills: Skill[];
  /** Ids checked or in force, drawn with the check. */
  selected: string[];
  /** Ids already recorded in the conversation, drawn as locked. */
  applied: string[];
  loading: boolean;
  onToggle: (id: string) => void;
}) {
  return (
    <div
      role="group"
      aria-label="Skills"
      className="absolute bottom-0 left-full z-50 w-[min(276px,calc(100vw-48px))] rounded-xl border border-[var(--line)] bg-[var(--rail)] py-1.5 pl-5 pr-1.5 shadow-2xl"
    >
      {loading && skills.length === 0 ? (
        <p className="px-2.5 py-2 text-[13px] text-[var(--quiet)]">Loading…</p>
      ) : skills.length === 0 ? (
        <p className="px-2.5 py-2 text-[13px] leading-5 text-[var(--quiet)]">
          No skills yet. Add one in Settings → Skills.
        </p>
      ) : (
        <ul className="max-h-72 overflow-y-auto">
          {skills.map((skill) => {
            const isSelected = selected.includes(skill.id);
            const isApplied = applied.includes(skill.id);
            return (
              <li key={skill.id}>
                <button
                  type="button"
                  role="checkbox"
                  aria-checked={isSelected}
                  disabled={isApplied}
                  onClick={() => onToggle(skill.id)}
                  title={skill.description ?? skill.instructions}
                  className="flex w-full items-center gap-2 rounded-lg py-1.5 pl-2.5 pr-2 text-left hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)] disabled:cursor-default disabled:hover:bg-transparent"
                >
                  <span className="min-w-0 flex-1">
                    <span className="flex items-center gap-1.5">
                      <span
                        className={`truncate text-[13px] ${
                          isApplied ? "text-[var(--quiet)]" : "text-[var(--text)]"
                        }`}
                      >
                        {skill.name}
                      </span>
                      {isApplied && (
                        <span className="shrink-0 text-[10px] text-[var(--quiet)]">in this chat</span>
                      )}
                      {skill.origin === "generated" && (
                        // Marked because a drafted skill encodes an assumption
                        // nobody checked, and this is the last place a user looks
                        // before sending instructions they did not write.
                        <Sparkles
                          size={11}
                          className="shrink-0 text-[var(--quiet)]"
                          aria-label="Drafted by agent"
                        />
                      )}
                    </span>
                    {skill.description && (
                      <span className="mt-0.5 block truncate text-[11px] leading-4 text-[var(--quiet)]">
                        {skill.description}
                      </span>
                    )}
                  </span>
                  {/* The check sits on the right and always occupies its slot. An
                      indicator rendered only when on shifts the text as rows are
                      checked, which reads as the list moving under the pointer. */}
                  <Check
                    size={15}
                    strokeWidth={2.5}
                    aria-hidden
                    className={`shrink-0 transition-colors ${
                      isSelected ? "text-[var(--accent)]" : "text-transparent"
                    }`}
                  />
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}