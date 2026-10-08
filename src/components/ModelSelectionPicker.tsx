import { useEffect, useMemo, useRef, useState } from "react";
import { ChevronDown, Cpu, Search } from "lucide-react";
import type { ModelGroup } from "../features/models/useModelRegistry";
import type { LocalModel, ModelSelection } from "../features/models/types";

/**
 * The pieces every model-*choice* picker in the app shares.
 *
 * There are two of them -- the title chain and the sub-agent default -- and they
 * ask the same question of the same data: which models can be chosen, and how is
 * one named. Keeping the option list and the identity of a selection in one place
 * is what stops the two drifting into disagreeing about, say, whether a local
 * model is keyed by `local:<id>` or its raw id -- a rule that works until someone
 * adds a second local model and the two copies have quietly diverged.
 *
 * (`components/ModelPicker.tsx` is a different, older control: a provider dropdown
 * plus a free-text model field. It is unrelated to this one, which is why the two
 * do not share a name.)
 */

/** A selection's stable identity, for keys and for finding it in the list. */
export const selectionId = (selection: ModelSelection) =>
  selection.localModelId
    ? `local:${selection.localModelId}`
    : `${selection.endpointId ?? ""}:${selection.model ?? ""}`;

export type ModelOption = {
  selection: ModelSelection;
  label: string;
  group: string;
  local: boolean;
};

/**
 * Every choosable model: remote providers first, local last.
 *
 * Built from the provider groups the caller already loaded rather than fetched
 * here, so a picker and the settings page beside it cannot end up offering two
 * different sets of models. `groups` already excludes switched-off models.
 */
export function useModelOptions(groups: ModelGroup[], localModels: LocalModel[]): ModelOption[] {
  return useMemo(
    () => [
      ...groups.flatMap((group) =>
        group.models.map((model) => ({
          selection: { endpointId: group.endpoint.id, model },
          label: model,
          group: group.endpoint.name,
          local: false,
        })),
      ),
      ...localModels.map((model) => ({
        selection: { localModelId: model.id },
        label: model.name,
        group: "Local",
        local: true,
      })),
    ],
    [groups, localModels],
  );
}

/** A selection's name for display, or `null` when it names nothing. */
export function selectionLabel(
  selection: ModelSelection | null | undefined,
  localModels: LocalModel[],
): string | null {
  if (!selection) return null;
  if (selection.localModelId) {
    return localModels.find((model) => model.id === selection.localModelId)?.name ?? null;
  }
  return selection.model ?? null;
}

type SingleModelPickerProps = {
  /** The chosen model, or `null` for the caller's "no choice" state. */
  value: ModelSelection | null;
  onChange: (selection: ModelSelection | null) => void;
  groups: ModelGroup[];
  localModels: LocalModel[];
  /** Shown when nothing is chosen. Names what that state means, not "none". */
  emptyLabel: string;
  /** Accessible name for the control. */
  label: string;
};

/**
 * Picking one model.
 *
 * The single-select sibling of the title picker, and a separate component rather
 * than a mode of it because the two differ in the one behaviour that matters: the
 * title picker adds a model to an ordered chain, and this one replaces the
 * current choice. Sharing the control would mean a prop that switches between
 * "add" and "replace", which is two controls pretending to be one.
 */
export default function SingleModelPicker({
  value,
  onChange,
  groups,
  localModels,
  emptyLabel,
  label,
}: SingleModelPickerProps) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const dismiss = (event: PointerEvent) => {
      if (containerRef.current && !containerRef.current.contains(event.target as Node)) setOpen(false);
    };
    const onKeyDown = (event: KeyboardEvent) => { if (event.key === "Escape") setOpen(false); };
    document.addEventListener("pointerdown", dismiss);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("pointerdown", dismiss);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, []);

  const options = useModelOptions(groups, localModels);
  const normalized = query.trim().toLowerCase();
  const filtered = options.filter((option) => `${option.label} ${option.group}`.toLowerCase().includes(normalized));
  const grouped = filtered.reduce<Map<string, typeof filtered>>((map, option) => {
    const bucket = map.get(option.group) ?? [];
    bucket.push(option);
    map.set(option.group, bucket);
    return map;
  }, new Map());

  const currentId = value ? selectionId(value) : null;
  const triggerLabel = selectionLabel(value, localModels) ?? emptyLabel;

  const choose = (selection: ModelSelection | null) => {
    onChange(selection);
    setOpen(false);
  };

  return (
    <div ref={containerRef} className="relative shrink-0">
      <button
        type="button"
        onClick={() => setOpen((current) => !current)}
        aria-expanded={open}
        aria-label={label}
        className="inline-flex h-9 w-[190px] items-center justify-between gap-2 rounded-md border border-[var(--line)] bg-[var(--rail)] px-3 text-[13px] text-[var(--text)] transition-colors hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
      >
        <span className={`min-w-0 flex-1 truncate text-left ${value ? "" : "text-[var(--quiet)]"}`}>{triggerLabel}</span>
        <ChevronDown size={14} className="shrink-0 text-[var(--muted)]" />
      </button>
      {open && <div className="absolute right-0 top-full z-40 mt-1 flex max-h-[min(60vh,400px)] w-[min(300px,70vw)] flex-col overflow-hidden rounded-lg border border-[var(--line)] bg-[var(--rail)] shadow-2xl">
        <label className="flex min-h-10 items-center gap-2 border-b border-[var(--line)] px-3 text-[var(--muted)] focus-within:outline-none">
          <Search size={15} className="shrink-0" />
          <span className="sr-only">Search models</span>
          <input autoFocus value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search models" className="min-w-0 flex-1 bg-transparent text-[13px] text-[var(--text)] outline-none placeholder:text-[var(--quiet)]" />
        </label>
        <div className="min-h-0 flex-1 overflow-y-auto p-1.5">
          {/* The "no choice" row, first. It is a real answer rather than a way to
              clear a field, which is why it reads as the state it selects. */}
          <button
            type="button"
            onClick={() => choose(null)}
            aria-pressed={value === null}
            className="flex min-h-9 w-full items-center gap-2 rounded-md px-2 text-left text-[13px] text-[var(--muted)] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-[var(--accent)]"
          >
            <span className={`min-w-0 flex-1 truncate ${value === null ? "font-medium text-[var(--text)]" : ""}`}>{emptyLabel}</span>
          </button>
          {[...grouped.entries()].map(([group, groupOptions]) => <section key={group} className="mt-1 first:mt-0">
            <h4 className="px-2 pb-1 pt-1 text-[10px] font-semibold text-[var(--quiet)]">{group.toUpperCase()}</h4>
            {groupOptions.map((option) => {
              const id = selectionId(option.selection);
              const selected = id === currentId;
              return <button
                key={id}
                type="button"
                onClick={() => choose(option.selection)}
                aria-pressed={selected}
                className="group flex min-h-9 w-full items-center gap-2 rounded-md px-2 text-left text-[13px] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-[var(--accent)]"
              >
                {option.local && <Cpu size={14} className="shrink-0 text-[var(--quiet)]" />}
                <span className={`min-w-0 flex-1 truncate ${selected ? "font-medium text-[var(--text)]" : "text-[var(--muted)]"}`}>{option.label}</span>
              </button>;
            })}
          </section>)}
          {filtered.length === 0 && <p className="px-2 py-5 text-sm text-[var(--muted)]">{options.length ? "No models match that search." : "Add a provider or local model first."}</p>}
        </div>
      </div>}
    </div>
  );
}
