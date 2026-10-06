import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { LocalModel } from "./types";
import type { LocalRuntimeSettings } from "../../components/RuntimeSettingsPage";

/**
 * Which local models have a projector configured, keyed by model id.
 *
 * A projector (`mmproj`) is what lets a local model read images, so it is the
 * local answer to the vision capability a remote provider reports in
 * `/models`. It lives in the model's own runtime settings rather than in the
 * model row, because it is a per-model setting the user can attach and detach
 * and the model file itself knows nothing about it.
 *
 * **A model with no settings is in the map as `false`, not absent.** Absent
 * would mean "we have not asked yet", and the badge that reads it cannot tell
 * those apart from "asked, and it cannot". The stored settings carry no
 * projector by default, so a first read already means no vision.
 *
 * Never rejects: a model that cannot be read, or a database that predates the
 * settings table, contributes `false` — the normal case rather than an error.
 */
export async function loadLocalProjectors(
  models: LocalModel[],
): Promise<Record<string, boolean>> {
  const entries = await Promise.all(
    models.map(async (model) => {
      try {
        const settings = await invoke<LocalRuntimeSettings | null>("local_runtime_settings", {
          modelId: model.id,
        });
        return [model.id, Boolean(settings?.mmproj?.trim())] as const;
      } catch {
        return [model.id, false] as const;
      }
    }),
  );
  return Object.fromEntries(entries);
}

/**
 * Projector state for the local models currently listed.
 *
 * Refetched only when the model list itself changes, so opening the picker
 * reads nothing and switching the search text never triggers a command.
 */
export function useLocalProjectors(models: LocalModel[]): Record<string, boolean> {
  const [projectors, setProjectors] = useState<Record<string, boolean>>({});
  const signature = models.map((model) => model.id).join("~");

  useEffect(() => {
    let alive = true;
    void loadLocalProjectors(models).then((loaded) => {
      if (alive) setProjectors(loaded);
    });
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [signature]);

  return projectors;
}