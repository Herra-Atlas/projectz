/**
 * The settings window's pages, as data.
 *
 * `SettingsPage` used to live in `SettingsModal.tsx`, which meant anything that needed to
 * name a page -- the search index, its tests -- had to import the whole modal.
 * It is one union and it changes rarely, so it lives on its own and the modal imports it.
 *
 * A drill-down page (`model`, `form`) carries the record it is about. Those are
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
  /* Creating one voice: prompted takes, then a name. A sub page of the wake and
     voice-recognition stage inside Speech to text, with its own back, because it
     is a sequence rather than a setting. */
  | { view: "voiceEnroll" }
  /* The audio.cpp engine, beside the two native ones: a second voice set and a
     second recognition set, installed the way the llama.cpp engines are. */
  | { view: "audio" }
  /* Audio.cpp's models, reached from the engine page: they serve both speech
     and recognition, so they belong to neither of the two lists. */
  | { view: "audioModels" }
  /* One audio.cpp model's options, carried by its model path and named by its
     family's spec. `from` says which list opened it, because the same record
     is reached from the wake stage and from the audio models page. */
  | { view: "audioModel"; model: string; from?: "wake" }
  | { view: "general" }
  | { view: "appearance" }
  | { view: "taste" }
  /* The Model section: what the assistant talks to, how hard it may work, and
     how context is kept inside the window. Four pages so a reader tuning them
     does not scroll unrelated rows to do it. */
  | { view: "mainModel" }
  | { view: "subagents" }
  | { view: "auxiliary" }
  | { view: "context" }
  /* Messaging to the agent from outside the app. A group, like Voice and Model:
     the connection, how it answers, and the access boundary are three pages
     rather than one long scroll. */
  | { view: "communicationsTelegram" }
  | { view: "communicationsSettings" }
  | { view: "communicationsSecurity" }
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
  | { view: "model"; model: LocalModel }
  | { view: "form"; endpoint: Endpoint | null };
