import { useEffect, useState } from "react";
import { Pencil, Trash2 } from "lucide-react";
import type { TasteItem } from "../../features/taste/types";
import {
  SECTION_LABELS,
  SECTION_ORDER,
  confidenceLabel,
  seenLabel,
} from "../../features/taste/tasteFormat";

type TasteDocumentProps = {
  items: TasteItem[];
  /** Rewrites one fact's text. Rejects when the backend refused it. */
  onUpdate: (id: number, text: string) => Promise<void>;
  /** Deletes one fact. Rejects when the backend refused it. */
  onDelete: (id: number) => Promise<void>;
};

/**
 * The profile as the settings page draws it.
 *
 * Grouped by the same fixed headings the prompt uses, in the same order. A fact is
 * editable in place: an inferred statement about a person should be correctable
 * without waiting for the next model run, and deleting one should be as direct as
 * deleting a provider.
 */
export default function TasteDocument({ items, onUpdate, onDelete }: TasteDocumentProps) {
  const [editingId, setEditingId] = useState<number | null>(null);
  const [draft, setDraft] = useState("");
  const [saving, setSaving] = useState(false);

  // An item can disappear underneath an open editor -- a run finished, or a
  // second window deleted it. Dropping the editor then is better than leaving it
  // pointing at an id that no longer exists.
  useEffect(() => {
    if (editingId !== null && !items.some((item) => item.id === editingId)) {
      setEditingId(null);
      setSaving(false);
    }
  }, [items, editingId]);

  if (items.length === 0) {
    return <p className="px-4 py-5 text-[12px] text-[var(--quiet)]">Nothing learned yet.</p>;
  }

  const startEdit = (item: TasteItem) => {
    setEditingId(item.id);
    setDraft(item.text);
  };

  const save = async () => {
    if (editingId === null || saving) return;
    setSaving(true);
    try {
      await onUpdate(editingId, draft);
      setEditingId(null);
    } catch {
      // The page has already reported the reason. Keeping the editor open leaves
      // the text the user wrote where they can correct it.
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="px-4 py-4">
      <div className="space-y-5">
        {SECTION_ORDER.map((section) => {
          const sectionItems = items.filter((item) => item.section === section);
          if (sectionItems.length === 0) return null;
          return (
            <section key={section}>
              <h4 className="mb-2 text-[11px] font-semibold text-[var(--quiet)]">
                {SECTION_LABELS[section]}
              </h4>
              <ul className="space-y-1">
                {sectionItems.map((item) => {
                  const editing = editingId === item.id;
                  return (
                    <li key={item.id} className="flex min-h-9 items-start gap-3 py-1">
                      <span
                        className="mt-[7px] size-1.5 shrink-0 rounded-full bg-[var(--accent)]"
                        aria-hidden="true"
                      />
                      <div className="min-w-0 flex-1">
                        {editing ? (
                          <input
                            autoFocus
                            value={draft}
                            maxLength={220}
                            aria-label="Edit fact"
                            onChange={(event) => setDraft(event.target.value)}
                            onKeyDown={(event) => {
                              if (event.key === "Enter") {
                                event.preventDefault();
                                void save();
                              }
                              if (event.key === "Escape") setEditingId(null);
                            }}
                            className="h-8 w-full rounded-md border border-[var(--line)] bg-[var(--rail)] px-2.5 text-[13px] text-[var(--text)] outline-none focus:border-[var(--accent)]"
                          />
                        ) : (
                          <p className="text-[13px] leading-5 text-[var(--text)]">{item.text}</p>
                        )}
                        {!editing && (
                          <p className="mt-0.5 text-[10px] text-[var(--quiet)]">
                            {confidenceLabel(item.confidence)} confidence · last seen{" "}
                            {seenLabel(item.lastSeenAt)}
                          </p>
                        )}
                      </div>

                      {editing ? (
                        <div className="flex shrink-0 items-center gap-1">
                          <button
                            type="button"
                            onClick={() => void save()}
                            disabled={saving || draft.trim().length < 8}
                            className="min-h-8 rounded-md bg-[var(--accent)] px-2.5 text-[12px] font-medium text-[var(--accent-ink)] transition-opacity hover:opacity-90 disabled:opacity-50"
                          >
                            Save
                          </button>
                          <button
                            type="button"
                            onClick={() => setEditingId(null)}
                            disabled={saving}
                            className="min-h-8 rounded-md px-2 text-[12px] text-[var(--muted)] hover:bg-[var(--raised)] disabled:opacity-50"
                          >
                            Cancel
                          </button>
                        </div>
                      ) : (
                        <div className="flex shrink-0 items-center">
                          <button
                            type="button"
                            onClick={() => startEdit(item)}
                            aria-label={`Edit fact: ${item.text}`}
                            title="Edit"
                            className="grid size-8 place-items-center rounded-md text-[var(--quiet)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
                          >
                            <Pencil size={14} />
                          </button>
                          <button
                            type="button"
                            onClick={() => void onDelete(item.id).catch(() => undefined)}
                            aria-label={`Delete fact: ${item.text}`}
                            title="Delete"
                            className="grid size-8 place-items-center rounded-md text-[var(--quiet)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--danger)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
                          >
                            <Trash2 size={14} />
                          </button>
                        </div>
                      )}
                    </li>
                  );
                })}
              </ul>
            </section>
          );
        })}
      </div>
    </div>
  );
}
