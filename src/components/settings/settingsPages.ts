/**
 * The settings window's pages, as data.
 *
 * `SettingsPage` used to live in `SettingsModal.tsx`, which meant anything that needed to
 * name a page -- the search index, its tests -- had to import the whole modal.
 * It is one union and it changes rarely, so it lives on its own and the modal imports it.
 *
 * A drill-down page (`model`, `skill`, `form`) carries the record it is about. Those are
 * reached from their list pages, not from search.
 */
import type { Endpoint, LocalModel } from "../../features/models/types";
import type { DownloadKind } from "../../features/downloads/types";

export type SettingsPage =
  | { view: "providers" }
  | { view: "local" }
  | { view: "engines" }
  /* Speech, its own section rather than a half of the Local page. The voices
     (synthesis) and the models (recognition) answer a different question than
     the GGUF models do, so they are two pages, each carrying its own settings
     and its own installs. */
  | { view: "stt" }
  | { view: "tts" }
  | { view: "general" }
  | { view: "appearance" }
  | { view: "assist" }
  | { view: "taste" }
  /* The Model section: what the assistant talks to, how hard it may work, and
     how context is kept inside the window. Four pages so a reader tuning them
     does not scroll unrelated rows to do it. */
  | { view: "mainModel" }
  | { view: "subagents" }
  | { view: "auxiliary" }
  | { view: "context" }
  /* Messaging to the agent from outside the app. One page for every channel:
     Telegram is the first, and a second transport is another section rather than
     another sidebar entry. */
  | { view: "communications" }
  | { view: "skills" }
  | { view: "mcp" }
  /* A download page is reached from the engine it installs for, so it carries
     that engine rather than asking again on the page itself. */
  | { view: "download"; kind: DownloadKind }
  /* One voice's own settings, carried by the voice's model path. */
  | { view: "ttsVoice"; voice: string }
  /* A recognition model's own settings: what language it should expect. */
  | { view: "sttModel"; model: string }
  /* Transfers in flight, and what finished while the app was open. */
  | { view: "downloads" }
  /* What this installation is, what it uses, and the one destructive action. */
  | { view: "system" }
  | { view: "skill"; slug: string | null }
  | { view: "model"; model: LocalModel }
  | { view: "form"; endpoint: Endpoint | null };
