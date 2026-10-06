import { invoke } from "@tauri-apps/api/core";

/** What a provider told us one of its models can do.
 *
 * Every field is optional, and `undefined` means the provider never said. That
 * is deliberately different from `false` or an empty list: an unknown model must
 * keep behaving exactly as it did before capabilities existed, rather than being
 * read as unable to do something. */
export type ModelCapabilities = {
  context_length?: number | null;
  input_modalities?: string[] | null;
  output_modalities?: string[] | null;
  supports_reasoning?: boolean | null;
  reasoning_values?: string[] | null;
  supports_tools?: boolean | null;
};

/** Assumed context window for a model that has never been inspected.
 *
 * Only a fallback for the composer's ring. A real figure from the provider
 * always wins, even a smaller one, so a small local model is not shown with a
 * generous window.
 *
 * This is the *only* copy of the number. There is deliberately no Rust
 * counterpart: `None` from a provider means *unknown*, and a fallback applied
 * in the parser would turn every untested model into one reporting a 200K
 * window -- drawing a badge for something nobody ever claimed. Choosing what to
 * assume is a decision made when the window is drawn, not when it is parsed. */
export const DEFAULT_CONTEXT_LENGTH = 200_000;

/** The effort levels offered when nothing is known about the model. */
export const DEFAULT_EFFORT_VALUES = ["none", "low", "medium", "high"];

/**
 * Reads a model's capabilities from the local cache.
 *
 * This never throws and never rejects: a provider that was never tested has no
 * capabilities, which is the normal case rather than an error. Callers get
 * `undefined` fields and fall back to their previous behaviour.
 */
export async function loadModelCapabilities(
  endpointId: string | null,
  model: string | null,
): Promise<ModelCapabilities | null> {
  if (!endpointId || !model) return null;
  try {
    return await invoke<ModelCapabilities>("ai_model_capabilities", {
      endpointId,
      model,
    });
  } catch {
    // A model removed from the provider, or a database that predates this
    // table. Either way there is nothing to adapt to.
    return null;
  }
}

/** Context window to assume for a model, or `null` when the model is local. */
export function contextLengthFor(capabilities: ModelCapabilities | null): number | null {
  const reported = capabilities?.context_length;
  return typeof reported === "number" && reported > 0 ? reported : null;
}

/**
 * Effort levels this model accepts, in the provider's own vocabulary.
 *
 * A provider spells them differently (`none` on one, `off` on another) and the
 * difference is exactly why a hardcoded level is rejected with HTTP 400. When
 * the provider said nothing, the previous four levels are used.
 */
export function effortValuesFor(capabilities: ModelCapabilities | null): string[] {
  const reported = capabilities?.reasoning_values;
  if (reported && reported.length > 0) return reported;
  return DEFAULT_EFFORT_VALUES;
}

/** True when the model takes no reasoning control at all.
 *
 * Only an explicit `false` counts. Unknown or missing keeps the control
 * available, because hiding it on a provider we have not inspected would break
 * a model that works today.
 */
export function reasoningSupported(capabilities: ModelCapabilities | null): boolean {
  return capabilities?.supports_reasoning !== false;
}

/** True when the model can be sent a `tools` array.
 *
 * Only an explicit `false` counts, for the same reason as `reasoningSupported`:
 * unknown keeps Agent mode offered. Hiding it on an untested provider would
 * remove a working feature from every model we have not inspected.
 *
 * An explicit `false` is worth honouring though, because the failure it prevents
 * is silent. A model sent tools it ignores does not error -- it answers in prose,
 * and the user sees a mode that appears to do nothing at all.
 */
export function toolsSupported(capabilities: ModelCapabilities | null): boolean {
  return capabilities?.supports_tools !== false;
}

/** True when the model reported that it can read this kind of input. */
export function acceptsInput(
  capabilities: ModelCapabilities | null,
  modality: "image" | "audio" | "video",
): boolean {
  const inputs = capabilities?.input_modalities;
  if (!inputs || inputs.length === 0) return true;
  return inputs.includes(modality);
}

/** Every known model of one provider, keyed by model id.
 *
 * One call per provider, never one per model: a single gateway can report
 * several hundred models, and asking for each one separately floods the command
 * queue with work the database can answer in a single read.
 *
 * Never throws. A provider that was never tested contributes no entry, which is
 * the normal case rather than an error, so a picker is never taken down by a
 * capability lookup. */
export async function loadProviderCapabilities(
  endpointId: string,
): Promise<Record<string, ModelCapabilities>> {
  if (!endpointId) return {};
  try {
    return await invoke<Record<string, ModelCapabilities>>("ai_provider_capabilities", { endpointId });
  } catch {
    return {};
  }
}

/** Capabilities for several providers at once, keyed by `"endpointId:model"`.
 *
 * One request per provider rather than one per model. Used by the lists that
 * show every provider's models at once. */
export async function loadManyCapabilities(
  endpointIds: string[],
  modelsByEndpoint: Record<string, string[]>,
): Promise<Record<string, ModelCapabilities>> {
  const unique = [...new Set(endpointIds.filter(Boolean))];
  const perProvider = await Promise.all(
    unique.map(async (endpointId) => [endpointId, await loadProviderCapabilities(endpointId)] as const),
  );
  const output: Record<string, ModelCapabilities> = {};
  for (const [endpointId, entries] of perProvider) {
    const models = modelsByEndpoint[endpointId];
    // Only carry over models the provider is about to show. kilocode stores
    // capabilities for hundreds of models, and copying all of them into a
    // lookup keyed for display would be wasted work.
    if (!models) continue;
    for (const model of models) {
      const capabilities = entries[model];
      if (capabilities) output[`${endpointId}:${model}`] = capabilities;
    }
  }
  return output;
}