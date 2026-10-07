/**
 * Asks before deleting, because this one is not undoable.
 *
 * The card list has no undo and a skill is something a user wrote by hand, so a
 * single stray click would lose real work. Disabling is always available as the
 * reversible option, which is what the copy points at.
 */
export default function SkillDeleteDialog({
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
        className="w-full max-w-sm rounded-xl border border-[var(--line)] bg-[var(--panel)] p-5 shadow-2xl"
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
