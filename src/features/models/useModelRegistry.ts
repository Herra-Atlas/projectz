import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Endpoint, LocalModel } from "./types";

const MODELS_CACHE_KEY = "projectz.models-cache.v1";

/** Composite key used across persisted settings to reference a remote model. */
export const modelKey = (endpointId: string, model: string) => `${endpointId}:${model}`;

export type ModelGroup = { endpoint: Endpoint; models: string[] };

/**
 * Loads providers and registered GGUF models.
 *
 * Providers are seeded from a localStorage cache so the picker paints instantly,
 * then refreshed from the backend whenever `refreshKey` changes. Shared by the
 * chat model switcher and the settings model fields.
 *
 * A switched-off model needs no filtering here: `ai_list_endpoints` already
 * leaves it out of `endpoint.models`, which is what the `enabled` column on
 * `provider_models` means. It used to be re-applied from an `app.disabled_models`
 * array on every read, and that second copy was only ever written by the
 * frontend -- so the column and the array could disagree, and the read believed
 * the array.
 */
export function useModelRegistry(refreshKey: number) {
  const [endpoints, setEndpoints] = useState<Endpoint[]>(() => {
    try { return JSON.parse(localStorage.getItem(MODELS_CACHE_KEY) ?? "[]") as Endpoint[]; }
    catch { return []; }
  });
  const [localModels, setLocalModels] = useState<LocalModel[]>([]);

  useEffect(() => {
    let mounted = true;
    Promise.all([
      invoke<Endpoint[]>("ai_list_endpoints"),
      invoke<LocalModel[]>("local_models_list"),
    ]).then(([providers, local]) => {
      if (!mounted) return;
      setEndpoints(providers);
      setLocalModels(local);
      localStorage.setItem(MODELS_CACHE_KEY, JSON.stringify(providers));
    }).catch(() => undefined);
    return () => { mounted = false; };
  }, [refreshKey]);

  // Remote models grouped by provider. Disabled providers and switched-off
  // models are already absent from `endpoint.models`, so this only has to drop a
  // provider with nothing left to show.
  const groups = useMemo(() => endpoints
    .filter((endpoint) => endpoint.enabled && endpoint.models.length > 0)
    .map((endpoint) => ({ endpoint, models: endpoint.models })),
  [endpoints]);

  return { endpoints, localModels, groups };
}