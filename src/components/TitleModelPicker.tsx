import { useEffect, useMemo, useRef, useState } from "react";
import { ChevronDown, Cpu, Plus, Search } from "lucide-react";
import { MAX_TITLE_MODELS } from "../features/models/usePreferences";
import type { ModelGroup } from "../features/models/useModelRegistry";
import type { LocalModel, ModelSelection } from "../features/models/types";

type TitleModelPickerProps = {
  /** Models in priority order. The first is tried first. */
  models: ModelSelection[];
  onChange: (models: ModelSelection[]) => void;
  groups: ModelGroup[];
  localModels: LocalModel[];
};

const selectionId = (selection: ModelSelection) =>
  selection.localModelId ? `local:${selection.localModelId}` : `${selection.endpointId ?? ""}:${selection.model ?? ""}`;

/**
 * Single-row control for the models that name new conversations.
 *
 * The row states what the setting does; the panel is a searchable list where
 * each chosen model carries its position as a number. The number is the only
 * thing that conveys priority, so it earns its place on the right.
 */
export default function TitleModelPicker({ models, onChange, groups, localModels }: TitleModelPickerProps) {
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

  const options = useMemo(() => [
    ...groups.flatMap((group) => group.models.map((model) => ({
      selection: { endpointId: group.endpoint.id, model },
      label: model,
      group: group.endpoint.name,
      local: false,
    }))),
    ...localModels.map((model) => ({
      selection: { localModelId: model.id },
      label: model.name,
      group: "Local",
      local: true,
    })),
  ], [groups, localModels]);

  const normalized = query.trim().toLowerCase();
  const filtered = options.filter((option) => `${option.label} ${option.group}`.toLowerCase().includes(normalized));
  const grouped = filtered.reduce<Map<string, typeof filtered>>((map, option) => {
    const bucket = map.get(option.group) ?? [];
    bucket.push(option);
    map.set(option.group, bucket);
    return map;
  }, new Map());

  // Position of each chosen model, so the list can show its order directly.
  const orderById = new Map(models.map((selection, index) => [selectionId(selection), index + 1]));
  const isFull = models.length >= MAX_TITLE_MODELS;

  const toggle = (selection: ModelSelection) => {
    const id = selectionId(selection);
    if (orderById.has(id)) onChange(models.filter((current) => selectionId(current) !== id));
    else if (!isFull) onChange([...models, selection]);
  };

  const triggerLabel = models.length === 0
    ? "None"
    : models.length === 1
      ? models[0].model ?? localModels.find((model) => model.id === models[0].localModelId)?.name ?? "1 model"
      : `${models.length} models`;

  return (
    <div ref={containerRef} className="flex min-h-[68px] items-center gap-3 border-b border-[var(--line)] py-3 last:border-b-0">
      <div className="min-w-0 flex-1">
        <h3 className="text-sm font-medium">Title generation</h3>
        <p className="text-xs text-[var(--quiet)]">Name new conversations after the first reply.</p>
      </div>
      <div className="relative shrink-0">
        <button type="button" onClick={() => setOpen((current) => !current)} aria-expanded={open} aria-label="Models used to generate session titles" className="inline-flex h-9 w-[150px] items-center justify-between gap-2 rounded-md border border-[var(--line)] bg-[var(--rail)] px-3 text-sm text-[var(--text)] transition-colors hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]">
          <span className={`min-w-0 flex-1 truncate text-left ${models.length === 0 ? "text-[var(--quiet)]" : ""}`}>{triggerLabel}</span>
          <ChevronDown size={14} className="shrink-0 text-[var(--muted)]" />
        </button>
        {open && <div className="absolute right-0 top-full z-40 mt-1 flex max-h-[min(60vh,400px)] w-[min(300px,70vw)] flex-col overflow-hidden rounded-lg border border-[var(--line)] bg-[var(--rail)] shadow-2xl">
          <label className="flex min-h-10 items-center gap-2 border-b border-[var(--line)] px-3 text-[var(--muted)] focus-within:outline-none">
            <Search size={15} className="shrink-0" />
            <span className="sr-only">Search models</span>
            <input autoFocus value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search models" className="min-w-0 flex-1 bg-transparent text-sm text-[var(--text)] outline-none placeholder:text-[var(--quiet)]" />
          </label>
          <div className="min-h-0 flex-1 overflow-y-auto p-1.5">
            {[...grouped.entries()].map(([group, groupOptions]) => <section key={group} className="mt-1 first:mt-0">
              <h4 className="px-2 pb-1 pt-1 text-[10px] font-semibold text-[var(--quiet)]">{group.toUpperCase()}</h4>
              {groupOptions.map((option) => {
                const id = selectionId(option.selection);
                const position = orderById.get(id);
                const selected = position !== undefined;
                return <button
                  key={id}
                  type="button"
                  onClick={() => toggle(option.selection)}
                  disabled={!selected && isFull}
                  aria-pressed={selected}
                  title={selected ? `Remove ${option.label} from position ${position}` : isFull ? `Only ${MAX_TITLE_MODELS} models are used` : `Use ${option.label} next`}
                  className="group flex min-h-9 w-full items-center gap-2 rounded-md px-2 text-left text-[13px] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-[var(--accent)] disabled:cursor-not-allowed disabled:opacity-40"
                >
                  {option.local && <Cpu size={14} className="shrink-0 text-[var(--quiet)]" />}
                  <span className={`min-w-0 flex-1 truncate ${selected ? "font-medium text-[var(--text)]" : "text-[var(--muted)]"}`}>{option.label}</span>
                  {selected
                    ? <span className="grid size-5 shrink-0 place-items-center rounded-full bg-[var(--accent)] text-[10px] font-semibold tabular-nums text-[var(--accent-ink)]" aria-label={`Position ${position}`}>{position}</span>
                    : <Plus size={14} className="shrink-0 text-[var(--quiet)] opacity-0 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100" />}
                </button>;
              })}
            </section>)}
            {filtered.length === 0 && <p className="px-2 py-5 text-sm text-[var(--muted)]">{options.length ? "No models match that search." : "Add a provider or local model first."}</p>}
          </div>
          <div className="flex items-center justify-between gap-2 border-t border-[var(--line)] px-3 py-2 text-[11px] text-[var(--quiet)]">
            <span>{isFull ? `Using ${MAX_TITLE_MODELS} of ${MAX_TITLE_MODELS}` : `${models.length} of ${MAX_TITLE_MODELS} used`}</span>
            {models.length > 0 && <button type="button" onClick={() => onChange([])} className="rounded px-1 hover:text-[var(--danger)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]">Clear</button>}
          </div>
        </div>}
      </div>
    </div>
  );
}
