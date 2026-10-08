import { Plus, Sparkles } from "lucide-react";

/**
 * Shown when no skill exists yet.
 *
 * Teaches the space rather than announcing emptiness: it says what a skill is
 * and offers the one action that fills the list, because a heading with nothing
 * under it answers none of the questions someone new to skills is asking.
 */
export default function SkillsEmptyState({ onCreate }: { onCreate: () => void }) {
  return (
    <div className="rounded-lg border border-dashed border-[var(--line)] px-6 py-12">
      <div className="flex flex-col items-center">
        <span className="mb-2 grid size-10 place-items-center rounded-full bg-[var(--raised)] text-[var(--quiet)]">
          <Sparkles size={18} />
        </span>
        <p className="text-center text-sm text-[var(--text)]">No skills yet</p>
        <p className="mt-1 max-w-sm text-center text-xs leading-5 text-[var(--quiet)]">
          A skill is a set of instructions kept for reuse — how you want code written, what to
          check before calling something done, what to avoid.
        </p>
        <button
          type="button"
          onClick={onCreate}
          className="mt-4 inline-flex min-h-9 items-center gap-1.5 rounded-lg border border-[var(--line)] bg-[var(--panel)] px-3 text-[13px] font-medium text-[var(--text)] transition-colors hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
        >
          <Plus size={14} />Write your first
        </button>
      </div>
    </div>
  );
}
