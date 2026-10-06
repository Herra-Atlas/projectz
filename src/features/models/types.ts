// Shared model/provider shapes. These mirror the Rust payloads in
// `src-tauri/src/ai/remote/types.rs` and `src-tauri/src/ai/local.rs`, so the
// full objects returned by `ai_list_endpoints` satisfy them directly.

export type Endpoint = {
  id: string;
  name: string;
  base_url: string;
  /** Models currently available, i.e. the ones not switched off. */
  models: string[];
  /**
   * Models the user has switched off for the pickers.
   *
   * Reported alongside rather than merged into `models` so a picker needs no
   * filtering while Settings can still show them and switch them back on.
   */
  disabled_models: string[];
  enabled: boolean;
  has_api_key: boolean;
};

export type LocalModel = {
  id: string;
  name: string;
  path: string;
  size_bytes: number;
  /**
   * Whether the file is still on disk, decided by the backend.
   *
   * Asked there rather than in the frontend because that is where the model is
   * already being read: two path checks in two languages is two answers, and they
   * disagreed -- the frontend reported files missing that loaded perfectly.
   */
  present: boolean;
  /** The weights' quantisation from the GGUF header: `Q4_K_M` and so on. */
  quantization?: string | null;
  /**
   * The context window the file declares, in tokens.
   *
   * The model's ceiling, not the context it is currently run at -- which is a
   * setting and can be set lower.
   */
  context_length?: number | null;
  /**
   * The engine this model runs with, already resolved by the backend.
   *
   * Resolved rather than raw because the answer is "this model's own pin, or the
   * global default", and the frontend has no business reproducing that order: it
   * is what `LocalModelManager::load` uses to pick the executable, so a second
   * copy here is a second answer to the same question. `null` means no engine at
   * all, which is a real state -- such a model cannot be loaded.
   */
  engine_id?: string | null;
};

/**
 * A model referenced by a persisted preference. Remote selections use
 * `endpointId` + `model` (the same pair the chat view uses), local selections
 * use `localModelId`. `null` means the feature is switched off.
 */
export type ModelSelection = {
  endpointId?: string;
  model?: string;
  localModelId?: string;
};

export const isLocalSelection = (selection: ModelSelection | null): selection is ModelSelection & { localModelId: string } =>
  Boolean(selection?.localModelId);
