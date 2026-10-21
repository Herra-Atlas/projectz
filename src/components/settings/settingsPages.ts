/**
 * The settings window's pages, as data.
 *
 * `SettingsPage` used to live in `SettingsModal.tsx`, which meant anything that needed to
 * name a page -- the search index, the dropdown, its tests -- had to import the whole modal.
 * It is one union and it changes rarely, so it lives on its own and the modal imports it.
 *
 * A drill-down page (`model`, `skill`, `form`) carries the record it is about. Those are
 * reached from their list pages, not from search.
 */
import type { Endpoint, LocalModel } from "../../features/models/types";
import type { Skill } from "../../features/skills/types";
import type { DownloadKind } from "../../features/downloads/types";

export type SettingsPage =
  | { view: "providers" }
  | { view: "local" }
  | { view: "engines" }
  | { view: "general" }
  | { view: "appearance" }
  | { view: "assist" }
  | { view: "taste" }
  | { view: "preferences" }
  | { view: "skills" }
  | { view: "mcp" }
  /* A download page is reached from the engine it installs for, so it carries
     that engine rather than asking again on the page itself. */
  | { view: "download"; kind: DownloadKind }
  /* Speech synthesis has settings of its own -- the voice and the pace -- and they
     are a page rather than a row because the engine list is already a picker. */
  | { view: "ttsSettings" }
  /* One voice's own settings. `null` is the voice the build placed, which is the
     same value the selection uses for it. */
  | { view: "ttsVoice"; voice: string | null }
  | { view: "skill"; skill: Skill | null }
  | { view: "model"; model: LocalModel }
  | { view: "form"; endpoint: Endpoint | null };
