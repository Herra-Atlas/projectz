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

/**
 * The stage of the Speech to text page that is showing.
 *
 * Named here, beside the pages, because the settings window holds it rather than
 * the page: the page unmounts whenever anything is opened from it.
 */
export type SpeechStage = "Voice recognition" | "Wake word" | "Speech to text";

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
     family's spec. */
  | { view: "audioModel"; model: string }
  | { view: "general" }
  | { view: "appearance" }
  /* Shortcuts the operating system owns. Its own page because it is the only
     setting that acts while another program is in front. */
  | { view: "keybinds" }
  /* What the app may interrupt for, and the sound each event makes. One page:
     the sound belongs beside the switch it belongs to, not on a page of its
     own where the two have to be kept in step by reading both. */
  | { view: "notifications" }
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
     that engine rather than asking again on the page itself. `group` is the
     section of the catalogue it opens on, when the page that sent the user here
     named one -- the audio.cpp download from Speech to text opens on
     `recognition`, from Text to speech on `speech`. */
  | { view: "download"; kind: DownloadKind; group?: string }
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

/**
 * What to call a page, for the Back control.
 *
 * The sidebar's own vocabulary: search already says which page a row is on with
 * these words, and a Back that called the same page something else would be a
 * third opinion about what it is.
 */
const PAGE_NAMES: Record<SettingsPage["view"], string> = {
  providers: "Providers",
  local: "Local models",
  engines: "Engines",
  stt: "Speech to text",
  tts: "Text to speech",
  voiceEnroll: "Enrolling a voice",
  audio: "Audio engine",
  audioModels: "audio.cpp models",
  audioModel: "Audio model settings",
  general: "General",
  appearance: "Appearance",
  keybinds: "Keybinds",
  notifications: "Notifications",
  taste: "Taste",
  mainModel: "Main model",
  subagents: "Subagents",
  auxiliary: "Auxiliary models",
  context: "Context",
  communicationsTelegram: "Telegram",
  communicationsSettings: "Communications settings",
  communicationsSecurity: "Communications security",
  skills: "Skills",
  mcp: "MCP servers",
  download: "Download",
  ttsVoice: "Voice settings",
  sttModel: "Recognition model settings",
  downloads: "Downloads",
  system: "System",
  model: "Local model settings",
  form: "Add provider",
};

export function pageName(page: SettingsPage): string {
  return PAGE_NAMES[page.view];
}
