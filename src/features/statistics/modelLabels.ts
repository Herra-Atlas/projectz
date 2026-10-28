import { displayModelName } from "../models/modelName";

/** The id pair a usage row carries for the model that answered.
 *
 * `model_id` is a routing key, not a label: a remote one is namespaced
 * (`vendor/name`), and a local one is the UUID `local_models` gave the file. The
 * page has to turn both into the name the user chose, which is why it needs the
 * registry beside the report. */
export type ModelIdentity = {
  model_id: string;
  provider_id: string;
};

/** What the page needs to name a model and its provider.
 *
 * Both maps are keyed by id, because that is all an old reply can carry: a
 * provider can be renamed and a local model deleted, and neither changes the
 * rows already written. */
export type ModelNames = {
  /** Endpoint id → the name the user gave it in Settings. */
  providers: Map<string, string>;
  /** Local model id → the GGUF file's own name. */
  local: Map<string, string>;
};

/** The alias the local server answers to, written as the model by an older reply
 * and by the runs adopted from a job or a channel. It names no model. */
const LOCAL_ALIAS = "projectz-local";

/**
 * Whether a reply came from the local runtime.
 *
 * Three signals, because no one of them covers every row. A local model's id is
 * in `names.local`; an older reply or an adopted run wrote the server alias
 * instead of a model; and a row with no provider cannot have come from one.
 *
 * The registry is what makes this reliable: choosing a local model does not
 * clear the selected endpoint, so a local reply can carry the id of a remote
 * provider it never used. Judging by the provider column alone would file it
 * under that provider.
 */
export const isLocalUsage = (usage: ModelIdentity, names: ModelNames) =>
  usage.model_id === LOCAL_ALIAS || names.local.has(usage.model_id) || !usage.provider_id;

/**
 * The model as a reader should see it.
 *
 * A local id is resolved through the model list, because the id is a UUID and
 * the name is the file's. A local model that has since been removed keeps its
 * row — the tokens were still spent — but its name is gone with the record, so
 * it is named for what it is rather than shown as an id nobody can place. A
 * remote id drops its routing prefix, exactly as the chat picker does.
 */
export function modelLabel(usage: ModelIdentity, names: ModelNames): string {
  if (isLocalUsage(usage, names)) return names.local.get(usage.model_id) ?? "Local model";
  return displayModelName(usage.model_id);
}

/**
 * The provider's name for the row's subline, or null when the row is local.
 *
 * An endpoint that no longer exists falls back to its stored id: the name lived
 * in Settings, and a removed provider cannot report it. `modelGroups` applies
 * the same rule to its view labels, so the subline and the tab that filters it
 * cannot name the same provider two ways.
 */
export function providerLabel(usage: ModelIdentity, names: ModelNames): string | null {
  if (isLocalUsage(usage, names)) return null;
  return names.providers.get(usage.provider_id) ?? usage.provider_id;
}
