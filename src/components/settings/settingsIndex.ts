import type { SettingsPage } from "./settingsPages";

/**
 * The views a search result can open.
 *
 * `model` and `skill` are reached from their list pages by choosing the record they are
 * about, so a search result cannot carry them. The provider form is the exception: "Add
 * provider" means *this* form with nothing in it, and a result that opened the list
 * instead made the user press the button the result was supposed to be. It is reachable
 * as a page on its own, which is why that view -- and only that one -- is included.
 */
/**
 * The views that are complete as `{ view }`.
 *
 * `model`, `skill`, `download` and `ttsVoice` need the record they are about, so
 * none can be named by a search result and none is a page the index lists. `form`
 * stays: opening it blank is exactly what "Add provider" means.
 */
export type SettingsView = Exclude<SettingsPage["view"], "model" | "skill" | "download" | "ttsVoice" | "sttModel">;

/**
 * One findable row, as search sees it.
 *
 * `anchor` is the value the row carries as `data-settings-anchor` on the page. It is a
 * stable name rather than a label because a label is the thing being searched -- changing
 * the words on a row must not silently break the scroll-to and the highlight.
 *
 * `keywords` is for the words a user would type that are not on screen: "dark mode" for
 * Theme, "temperature" for the local runtime, "proxy" for a provider. Search that only
 * matches what is already written down is search for someone who has already found it.
 */
export type SettingsEntry = {
  /** The page this row lives on. */
  view: SettingsView;
  /** The row's `data-settings-anchor` value. */
  anchor: string;
  /** What the row is called, as drawn. */
  label: string;
  /** The page's own name, so the dropdown can say where the row is. */
  page: string;
  /** Extra words that should find this row. */
  keywords: string[];
};

/**
 * One findable row, before it is given a page.
 *
 * `view` is optional and defaults to the page's own: most rows live on the page they are
 * listed under, and only an action that opens a *different* page -- "Add provider" opens
 * the form rather than the list -- has to say so.
 */
type IndexRow = Omit<SettingsEntry, "view" | "page"> & { view?: SettingsView };

/** A page of the index: its sidebar name and the rows it owns. */
type IndexPage = { view: SettingsView; name: string; entries: IndexRow[] };

/**
 * The index, in sidebar order so a dropdown reads like the window does.
 *
 * Every row here must exist on its page with the matching `data-settings-anchor`, and
 * every findable row should be here: an index that misses a row is worse than no search,
 * because it answers "nothing found" about a setting that is right there. The rows are
 * grouped by page, which is also the only place that grouping is written down.
 */
const PAGES: IndexPage[] = [
  {
    view: "general",
    name: "General",
    entries: [
      {
        anchor: "general.instructions",
        label: "Instructions for ProjectZ",
        keywords: ["system prompt", "custom instructions", "rules", "personality", "context"],
      },
      {
        anchor: "general.notifications",
        label: "Response completions",
        keywords: ["notification", "notify", "alert", "done", "finished"],
      },
      {
        anchor: "general.metrics",
        label: "Show token metrics",
        keywords: ["tokens", "usage", "stats", "per reply"],
      },
      {
        anchor: "general.sendWith",
        label: "Send with",
        keywords: ["enter", "ctrl", "keyboard", "shortcut", "newline"],
      },
      {
        anchor: "general.clearSessions",
        label: "Clear all sessions",
        keywords: ["delete chats", "wipe", "history", "reset conversations"],
      },
    ],
  },
  {
    view: "appearance",
    name: "Appearance",
    entries: [
      { anchor: "appearance.theme", label: "Theme", keywords: ["dark", "light", "system", "colours", "colors", "palette"] },
      { anchor: "appearance.accent", label: "Accent colour", keywords: ["color", "highlight", "tint", "brand"] },
    ],
  },
  {
    view: "preferences",
    name: "Preferences",
    entries: [
      {
        anchor: "preferences.titleModels",
        label: "Conversation titles",
        keywords: ["title model", "naming", "rename chat", "fallback"],
      },
      {
        anchor: "preferences.subagentModel",
        label: "Sub-agent model",
        keywords: ["subagent", "agent model", "delegate"],
      },
      {
        anchor: "preferences.visionModel",
        label: "Vision model",
        keywords: ["image", "picture", "screenshot", "read image", "multimodal"],
      },
      {
        anchor: "preferences.compaction",
        label: "Context compaction",
        keywords: ["trim", "long conversation", "context window", "summarise", "off", "fast"],
      },
    ],
  },
  {
    view: "assist",
    name: "Assist",
    entries: [
      {
        anchor: "assist.enabled",
        label: "Help without being asked",
        keywords: ["assist", "read page", "browser answers", "on", "off"],
      },
      {
        anchor: "assist.model",
        label: "The model that answers",
        keywords: ["assist model", "local", "provider"],
      },
      {
        anchor: "assist.setup",
        label: "Set up the extension",
        keywords: ["browser extension", "pair", "chrome", "install", "connect"],
      },
      {
        anchor: "assist.performance",
        label: "Performance",
        keywords: ["efficient", "normal", "high", "level", "preset", "cost"],
      },
      {
        anchor: "assist.subjects",
        label: "Worth more of your attention",
        keywords: ["subjects", "topics", "weight", "priorities"],
      },
      {
        anchor: "assist.hint",
        label: "Anything to keep in mind",
        keywords: ["hint", "preference", "prompt", "custom"],
      },
      {
        anchor: "assist.kinds",
        label: "What it may help with",
        keywords: ["kinds", "answer", "explain", "summarize", "translate", "check"],
      },
      {
        anchor: "assist.confidence",
        label: "Only speak when sure",
        keywords: ["confidence", "floor", "strict", "eager", "quiet"],
      },
      {
        anchor: "assist.thinking",
        label: "Let the model think before answering",
        keywords: ["reasoning", "thinking", "chain of thought"],
      },
      {
        anchor: "assist.minChars",
        label: "Shortest to look at",
        keywords: ["minimum length", "characters", "floor", "blocks"],
      },
      {
        anchor: "assist.candidates",
        label: "Candidates considered",
        keywords: ["candidates", "blocks", "short list"],
      },
      {
        anchor: "assist.perRequest",
        label: "Candidates per request",
        keywords: ["batch", "per request", "candidates"],
      },
      {
        anchor: "assist.answers",
        label: "Answers per reply",
        keywords: ["answers", "cards", "reply", "max"],
      },
      {
        anchor: "assist.tokens",
        label: "Token ceiling",
        keywords: ["tokens", "limit", "budget", "max tokens"],
      },
      {
        anchor: "assist.requests",
        label: "Requests per look",
        keywords: ["requests", "batches", "look"],
      },
      {
        anchor: "assist.perHour",
        label: "Answers an hour",
        keywords: ["hourly", "rate limit", "budget", "per hour"],
      },
      {
        anchor: "assist.dryRun",
        label: "Show me what it would have answered",
        keywords: ["dry run", "test", "preview", "debug"],
      },
      {
        anchor: "assist.usage",
        label: "This session",
        keywords: ["requests", "usage", "spent", "hour"],
      },
    ],
  },
  {
    view: "taste",
    name: "Taste",
    entries: [
      {
        anchor: "taste.enabled",
        label: "Learn about you",
        keywords: ["taste", "profile", "memory", "on", "off"],
      },
      { anchor: "taste.models", label: "Taste model", keywords: ["taste model", "learning model", "fallback"] },
      { anchor: "taste.interval", label: "Every", keywords: ["interval", "days", "cadence", "schedule", "frequency"] },
      { anchor: "taste.runNow", label: "Next run", keywords: ["run now", "learn now", "schedule", "next"] },
      { anchor: "taste.profile", label: "What ProjectZ knows", keywords: ["facts", "profile", "learned", "memory", "edit"] },
      { anchor: "taste.runs", label: "Runs", keywords: ["history", "reports", "past runs"] },
      { anchor: "taste.reset", label: "Forget the learned profile", keywords: ["reset", "delete", "forget", "clear memory"] },
    ],
  },
  {
    view: "skills",
    name: "Skills",
    entries: [
      { anchor: "skills.list", label: "Skills", keywords: ["skill", "instructions", "bundled", "generated", "managed"] },
    ],
  },
  {
    view: "mcp",
    name: "MCP",
    entries: [
      { anchor: "mcp.servers", label: "MCP servers", keywords: ["mcp", "model context protocol", "tools", "server"] },
    ],
  },
  {
    view: "providers",
    name: "Providers",
    entries: [
      { anchor: "providers.list", label: "Providers", keywords: ["api", "endpoint", "openai", "key", "model provider"] },
      {
        anchor: "providers.add",
        label: "Add provider",
        // The result opens the *form*, not the list -- see `SettingsView`.
        view: "form",
        keywords: ["new provider", "connect", "api key", "base url", "add endpoint", "register"],
      },
    ],
  },
  {
    view: "local",
    name: "Local",
    entries: [
      { anchor: "local.list", label: "Local models", keywords: ["gguf", "llama", "local", "download", "models"] },
      { anchor: "local.add", label: "Add models", keywords: ["add model", "gguf", "browse file"] },
    ],
  },
  {
    view: "engines",
    name: "Engines",
    entries: [
      { anchor: "engines.list", label: "Engines", keywords: ["llama.cpp", "runtime", "engine", "version", "install"] },
    ],
  },
];

/**
 * The rows search can find, in sidebar order.
 */
export function buildSettingsIndex(): SettingsEntry[] {
  return PAGES.flatMap((page) =>
    page.entries.map((entry) => ({ ...entry, view: entry.view ?? page.view, page: page.name })),
  );
}
