import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Check, ChevronDown, Cpu, Search, Star } from "lucide-react";
import { modelKey, useModelRegistry } from "../features/models/useModelRegistry";
import { displayModelName } from "../features/models/modelName";
import { loadManyCapabilities, type ModelCapabilities } from "../features/models/modelCapabilities";
import ModelBadges from "./modelBadges/ModelBadges";
import LocalModelBadges from "./modelBadges/LocalModelBadges";
import { useLocalProjectors } from "../features/models/useLocalProjectors";
import type { LocalModel } from "../features/models/types";

type ModelSwitcherProps = { selectedEndpoint: string; selectedModel: string; localModelId: string; refreshKey: number; onSelect: (endpointId: string, model: string) => void; onSelectLocal: (model: LocalModel) => void };
const SELECTED_MODEL_KEY = "projectz.selected-model.v1";
const FAVORITES_KEY = "app.model_favorites";

export default function ModelSwitcher({ selectedEndpoint, selectedModel, localModelId, refreshKey, onSelect, onSelectLocal }: ModelSwitcherProps) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [favorites, setFavorites] = useState<string[]>([]);
  const popoverRef = useRef<HTMLDivElement>(null);
  const { endpoints, localModels, groups } = useModelRegistry(refreshKey);
  const active = endpoints.find((endpoint) => endpoint.id === selectedEndpoint);

  // Capabilities for the models currently listed, read from the local cache in
  // one request per provider rather than one per model. Refreshed only when the
  // provider list itself changes, so opening the picker reads nothing.
  const [capabilitiesByModel, setCapabilitiesByModel] = useState<Record<string, ModelCapabilities>>({});
  const modelsByEndpoint = Object.fromEntries(groups.map((group) => [group.endpoint.id, group.models]));
  const capabilitySignature = groups.map((group) => `${group.endpoint.id}=${group.models.join(",")}`).join("~");
  useEffect(() => {
    let alive = true;
    void loadManyCapabilities(
      groups.map((group) => group.endpoint.id),
      modelsByEndpoint,
    ).then((loaded) => {
      if (alive) setCapabilitiesByModel(loaded);
    });
    return () => { alive = false; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [capabilitySignature]);

  // Local models have no provider to ask, so their projector state comes from
  // each model's own stored runtime settings. Same shape as above: one read per
  // local model, refreshed only when the local list itself changes.
  const localProjectors = useLocalProjectors(localModels);

  useEffect(() => {
    invoke<string[] | null>("database_get_setting", { key: FAVORITES_KEY })
      .then((saved) => setFavorites(saved ?? []))
      .catch(() => undefined);
  }, [refreshKey]);

  // Drop a now-unavailable selection so chat never targets a hidden or removed
  // model, and forget the cached pick when it points at something missing.
  useEffect(() => {
    const cached = JSON.parse(localStorage.getItem(SELECTED_MODEL_KEY) ?? "null") as { endpointId?: string; model?: string; localModelId?: string } | null;
    const { endpointId: cachedEndpoint, model: cachedModel, localModelId: cachedLocal } = cached ?? {};
    if (cachedEndpoint && cachedModel) {
      const available = groups.some((group) => group.endpoint.id === cachedEndpoint && group.models.includes(cachedModel));
      if (!available) localStorage.removeItem(SELECTED_MODEL_KEY);
    } else if (cachedLocal && !localModels.some((model) => model.id === cachedLocal)) {
      localStorage.removeItem(SELECTED_MODEL_KEY);
    }
    if (selectedEndpoint && !groups.some((group) => group.endpoint.id === selectedEndpoint && group.models.includes(selectedModel))) onSelect("", "");
  }, [groups, localModels, selectedEndpoint, selectedModel, onSelect]);

  useEffect(() => {
    const dismiss = (event: PointerEvent) => {
      if (popoverRef.current && !popoverRef.current.contains(event.target as Node)) setOpen(false);
    };
    const onKeyDown = (event: KeyboardEvent) => { if (event.key === "Escape") setOpen(false); };
    document.addEventListener("pointerdown", dismiss);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("pointerdown", dismiss);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, []);

  const toggleFavorite = (key: string) => {
    setFavorites((current) => {
      const next = current.includes(key) ? current.filter((item) => item !== key) : [...current, key];
      void invoke("database_set_setting", { key: "app.model_favorites", value: next }).catch(() => undefined);
      return next;
    });
  };

  // `groups` already excludes disabled providers and hidden models, so it can
  // drive both the favorites list and the per-provider sections directly.
  const normalizedQuery = query.trim().toLowerCase();
  const matches = (model: string, provider: string) => `${model} ${provider}`.toLowerCase().includes(normalizedQuery);
  const favoritesShown = groups.flatMap((group) => group.models
    .filter((model) => favorites.includes(modelKey(group.endpoint.id, model)) && matches(model, group.endpoint.name))
    .map((model) => ({ endpoint: group.endpoint, model, key: modelKey(group.endpoint.id, model) })));
  const grouped = groups
    .map((group) => ({
      endpoint: group.endpoint,
      entries: group.models
        .filter((model) => !favorites.includes(modelKey(group.endpoint.id, model)) && matches(model, group.endpoint.name))
        .map((model) => ({ endpoint: group.endpoint, model, key: modelKey(group.endpoint.id, model) })),
    }))
    .filter((group) => group.entries.length > 0);
  const filteredLocal = localModels.filter((model) => matches(model.name, "local"));

  const selectRemoteModel = (endpointId: string, model: string) => {
    onSelect(endpointId, model);
    localStorage.setItem(SELECTED_MODEL_KEY, JSON.stringify({ endpointId, model }));
    setOpen(false);
  };

  const selectLocalModel = (model: LocalModel) => {
    onSelectLocal(model);
    localStorage.setItem(SELECTED_MODEL_KEY, JSON.stringify({ localModelId: model.id }));
    setOpen(false);
  };

  const renderModel = (entry: { endpoint: { id: string; name: string }; model: string; key: string }) => (
    <div key={entry.key} className="group flex min-h-9 items-center gap-1 rounded-md px-1 hover:bg-[var(--raised)]">
      <button type="button" onClick={() => selectRemoteModel(entry.endpoint.id, entry.model)} className="flex min-w-0 flex-1 items-center gap-2 rounded px-1.5 py-1 text-left text-[13px]" aria-current={selectedEndpoint === entry.endpoint.id && selectedModel === entry.model ? "true" : undefined}>
        <span className="min-w-0 flex-1 truncate">{entry.model}</span>
        <ModelBadges capabilities={capabilitiesByModel[entry.key]} />
        {selectedEndpoint === entry.endpoint.id && selectedModel === entry.model && <Check size={14} className="shrink-0 text-[var(--accent)]" />}
      </button>
      <button type="button" onClick={() => toggleFavorite(entry.key)} className="grid size-8 shrink-0 place-items-center rounded text-[var(--quiet)] opacity-0 transition-opacity group-hover:opacity-50 group-focus-within:opacity-50 focus-visible:opacity-80 hover:!text-[var(--muted)] hover:!opacity-80" aria-label={favorites.includes(entry.key) ? `Remove ${entry.model} from favorites` : `Add ${entry.model} to favorites`} title={favorites.includes(entry.key) ? "Remove favorite" : "Add favorite"}>
        <Star size={14} fill={favorites.includes(entry.key) ? "currentColor" : "none"} />
      </button>
    </div>
  );

  return (
    <div ref={popoverRef} className="relative">
      <button type="button" onClick={() => setOpen((value) => !value)} aria-expanded={open} className="inline-flex h-6 max-w-[200px] items-center gap-1.5 rounded px-1.5 text-[10px] text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]">
        <span className="max-w-[165px] truncate" title={selectedModel || undefined}>{localModels.find((item) => item.id === localModelId)?.name || (selectedModel ? displayModelName(selectedModel) : active?.name || "Choose model")}</span>
        {/* The closed button has room for one figure, so only the context window
            appears here; the full set is in the list. */}
        {!localModelId && selectedModel && <ModelBadges capabilities={capabilitiesByModel[modelKey(selectedEndpoint, selectedModel)]} compact />}
        <ChevronDown size={11} />
      </button>
      {open && <div className="absolute bottom-full right-0 z-40 mb-2 flex max-h-[min(65vh,460px)] w-[min(340px,calc(100vw-88px))] flex-col overflow-hidden rounded-lg border border-[var(--line)] bg-[var(--rail)] shadow-2xl">
        <label className="flex min-h-10 items-center gap-2 border-b border-[var(--line)] px-3 text-[var(--muted)] focus-within:outline-none"><Search size={15} /><span className="sr-only">Search models</span><input autoFocus value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search models" className="min-w-0 flex-1 rounded-sm bg-transparent text-sm text-[var(--text)] outline-none placeholder:text-[var(--quiet)]" /></label>
        <div aria-label="Available models" className="min-h-0 overflow-y-auto p-2">
          {favoritesShown.length > 0 && <section><h3 className="px-2 pb-1 pt-1 text-[10px] font-semibold text-[var(--accent)]">FAVORITES</h3>{favoritesShown.map(renderModel)}</section>}
          {grouped.map(({ endpoint, entries: models }) => <section key={endpoint.id} className="mt-2"><h3 className="px-2 pb-1 pt-1 text-[10px] font-semibold text-[var(--quiet)]">{endpoint.name.toUpperCase()}</h3>{models.map(renderModel)}</section>)}
          {filteredLocal.length > 0 && <section className="mt-2"><h3 className="px-2 pb-1 pt-1 text-[10px] font-semibold text-[var(--quiet)]">LOCAL</h3>{filteredLocal.map((item) => <div key={item.id} className="flex min-h-9 items-center gap-2 rounded-md hover:bg-[var(--raised)]"><button type="button" onClick={() => selectLocalModel(item)} className="flex min-w-0 flex-1 items-center gap-2 rounded px-1.5 py-1 text-left text-[13px]" aria-current={localModelId === item.id ? "true" : undefined}><Cpu size={14} className="shrink-0 text-[var(--quiet)]" /><span className="min-w-0 flex-1 truncate">{item.name}</span><LocalModelBadges model={item} hasProjector={localProjectors[item.id] === true} />{localModelId === item.id && <Check size={14} className="shrink-0 text-[var(--accent)]" />}</button></div>)}</section>}
          {favoritesShown.length === 0 && grouped.length === 0 && filteredLocal.length === 0 && <p className="px-2 py-5 text-sm text-[var(--muted)]">{groups.length || localModels.length ? "No models match that search." : "Add a provider or local GGUF model in Settings."}</p>}
        </div>
      </div>}
    </div>
  );
}
