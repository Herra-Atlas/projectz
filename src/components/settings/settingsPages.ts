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
  | { view: "skill"; skill: Skill | null }
  | { view: "model"; model: LocalModel }
  | { view: "form"; endpoint: Endpoint | null };
