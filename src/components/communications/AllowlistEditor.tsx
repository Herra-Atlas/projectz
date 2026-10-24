import { useState } from "react";
import { Plus, Trash2 } from "lucide-react";

import { HoldToDelete } from "../HoldToDelete";
import type { Notify } from "../../features/notifications/types";

/**
 * The list of people the bot obeys.
 *
 * **Numeric ids, not usernames.** A Telegram username can be changed and later
 * re-registered by somebody else, so it is not an identity; the numeric id is. The
 * input accepts digits only and the backend keeps only those, so a hand-edited
 * record cannot smuggle in a second, mutable way to be somebody.
 *
 * An empty list is not "everyone" -- it is nobody. That is the safe direction for a
 * bot that can run tools on this machine, and the empty state says so rather than
 * leaving a reader to guess.
 */
export function AllowlistEditor({
  ids,
  onChange,
  notify,
}: {
  ids: string[];
  onChange: (ids: string[]) => void;
  notify: Notify;
}) {
  const [draft, setDraft] = useState("");

  const add = () => {
    const value = draft.trim();
    if (!/^\d+$/.test(value)) {
      notify("error", "A Telegram user id is numbers only.");
      return;
    }
    setDraft("");
    if (ids.includes(value)) return;
    onChange([...ids, value]);
  };

  return (
    <div className="flex flex-col gap-2.5">
      {ids.length === 0 ? (
        <p className="text-[12px] leading-5 text-[var(--quiet)]">
          Nobody is allowed yet, so the bot answers no one.
        </p>
      ) : (
        <ul className="flex flex-col gap-1">
          {ids.map((id) => (
            <li
              key={id}
              className="flex items-center justify-between gap-2 rounded-md bg-[var(--raised)] px-2.5 py-1.5"
            >
              <span className="text-[12px] tabular-nums">{id}</span>
              <HoldToDelete
                label={`Remove ${id}`}
                title="Hold to remove this id"
                onConfirm={() => onChange(ids.filter((existing) => existing !== id))}
                className="grid size-6 place-items-center rounded text-[var(--muted)] transition-colors hover:text-[var(--danger)]"
              >
                <Trash2 size={13} />
              </HoldToDelete>
            </li>
          ))}
        </ul>
      )}
      <div className="flex gap-2">
        <input
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              add();
            }
          }}
          inputMode="numeric"
          autoComplete="off"
          aria-label="Telegram user id"
          placeholder="Telegram user id"
          className="min-h-9 w-48 rounded-lg border border-[var(--line)] bg-[var(--panel)] px-2.5 text-[13px] tabular-nums text-[var(--text)] outline-none transition-colors focus:border-[var(--accent)]"
        />
        <button
          type="button"
          onClick={add}
          disabled={!draft.trim()}
          className="inline-flex min-h-9 items-center gap-1.5 rounded-lg border border-[var(--line)] bg-[var(--panel)] px-2.5 text-[13px] font-medium text-[var(--text)] transition-colors hover:border-[var(--line-strong)] hover:bg-[var(--raised)] disabled:opacity-40"
        >
          <Plus size={14} />
          Add
        </button>
      </div>
    </div>
  );
}
