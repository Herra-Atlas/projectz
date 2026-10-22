import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open as openFile } from "@tauri-apps/plugin-dialog";
import { ArrowLeft, BookMarked, Brain, ChevronDown, ChevronRight, Cpu, Download, GripVertical, Info, Lightbulb, LoaderCircle, MoveRight, Palette, Pencil, Plug, Plus, Search, Server, Settings2, SlidersHorizontal, Sparkles, Trash2, X, Zap } from "lucide-react";
import { resolveTheme, type Appearance } from "../features/appearance/appearance";
import AccentPicker from "./appearance/AccentPicker";
import ThemePicker from "./appearance/ThemePicker";
import ProviderIcon from "./ProviderIcon";
import ModelBadges from "./modelBadges/ModelBadges";
import EnginesSettingsPage from "./EnginesSettingsPage";
import { engineLabel } from "./EnginePicker";
import RuntimeSettingsPage, { type LocalRuntimeSettings } from "./RuntimeSettingsPage";
import EnginePicker from "./EnginePicker";
import SkillsPage from "./SkillsPage";
import McpSettingsPage from "./McpSettingsPage";
import AssistSettingsPage from "./assist/AssistSettingsPage";
import TasteSettingsPage from "./taste/TasteSettingsPage";
import SettingsSearch, { pageForEntry } from "./settings/SettingsSearch";
import { buildSettingsIndex, type SettingsEntry } from "./settings/settingsIndex";
import { LocalCategoryTabs, LOCAL_TITLES, type LocalCategory } from "./settings/local/LocalCategoryTabs";
import { LocalAddMenu } from "./settings/local/LocalAddMenu";
import { SpeechRecognitionCategory } from "./settings/local/SpeechRecognitionCategory";
import { SttModelSettingsPage } from "./settings/local/SttModelSettingsPage";
import { DownloadsPage } from "./settings/downloads/DownloadsPage";
import { SystemPage } from "./settings/SystemPage";
import { HoldToDelete } from "./HoldToDelete";
import type { DownloadJob } from "../features/downloads/types";
import { SpeechSynthesisCategory } from "./settings/local/SpeechSynthesisCategory";
import { TtsSettingsPage } from "./settings/local/TtsSettingsPage";
import { TtsVoiceSettingsPage } from "./settings/local/TtsVoiceSettingsPage";
import { DownloadPage } from "./settings/downloads/DownloadPage";
import type { SettingsPage } from "./settings/settingsPages";
import { useAssist } from "../features/assist/useAssist";
import { useTaste } from "../features/taste/useTaste";
import SkillDetailPage from "./skills/SkillDetailPage";
import TitleModelPicker from "./TitleModelPicker";
import SingleModelPicker from "./ModelSelectionPicker";
import { ANCHOR_ATTRIBUTE, FlashHighlight, MARKABLE_ATTRIBUTE, SettingsPageTitle, SettingsSearchScope, SettingsSection, SETTINGS_HIGHLIGHT_EVENT } from "./SettingsSection";
import { SettingRow, Segmented, Toggle } from "./SettingsSection";
import { COMPACTION_LABELS, type Compaction, type Preferences } from "../features/models/usePreferences";
import type { Endpoint, LocalModel } from "../features/models/types";
import type { InstalledEngine } from "../features/models/useInstalledEngines";
import { useClaimNotificationHost } from "../features/notifications/notificationHost";
import type { Notify } from "../features/notifications/types";
import {
  loadManyCapabilities,
  type ModelCapabilities,
} from "../features/models/modelCapabilities";

/**
 * A path reduced to something readable.
 *
 * The stored path carries Windows' `\\?\` device prefix, which is an
 * instruction to the filesystem layer and means nothing to a person reading a
 * settings list. Dropped for display only -- the full path stays on the tooltip
 * and is what is sent to the backend, because stripping it there would change
 * which file is opened.
 */
function fileLabel(path: string): string {
  return path.replace(/^\\\\\?\\/, "");
}

type EndpointDraft = { id: string; name: string; base_url: string; api_key: string; models: string[]; enabled: boolean };
type SettingsModalProps = { open: boolean; onClose: () => void; onEndpointsChanged: () => void; onClearSessions: () => Promise<number>; preferences: Preferences; onPreferencesChange: (preferences: Preferences) => void; appearance: Appearance; onAppearanceChange: (appearance: Appearance) => void; notify: Notify };
// The page union lives in `settings/settingsPages.ts` so the search index can name a page
// without importing this modal. Re-exported here for the components that already imported it.
export type { SettingsPage } from "./settings/settingsPages";

type GeneralSettings = {
  instructions: string;
  responseNotifications: boolean;
  showMetrics: boolean;
  sendWith: "Enter" | "Ctrl + Enter";
};

const DEFAULT_GENERAL: GeneralSettings = { instructions: "", responseNotifications: false, showMetrics: true, sendWith: "Enter" };

/** Fills in defaults for any key the stored record predates. */
const withGeneralDefaults = (saved: Partial<GeneralSettings> | null | undefined): GeneralSettings => ({ ...DEFAULT_GENERAL, ...(saved ?? {}) });

const INSTRUCTION_EXAMPLES = [
  "Ask clarifying questions before giving detailed answers.",
  "Keep explanations brief and to the point.",
  "I primarily code in Python and am not a coding beginner.",
];

const blankEndpoint = (): EndpointDraft => ({ id: crypto.randomUUID(), name: "", base_url: "", api_key: "", models: [], enabled: true });

/**
 * How far the pointer must travel before a press counts as a drag.
 *
 * Without a threshold, every click on a row would begin a drag and a drop on the
 * row's own group -- so a plain press would occasionally move a model nobody
 * meant to move. Four pixels is below the hand's natural wobble during a click
 * and above what a click produces.
 */
const DRAG_THRESHOLD = 4;

/**
 * How long a row a search landed on stays marked.
 *
 * Long enough to find with the eye after a page change, short enough that it is gone by the
 * time the row is being read rather than looked for. The page title keeps the colour for
 * the same time, so the two agree about what "here" means.
 */
const HIGHLIGHT_MS = 2000;

export default function SettingsModal({ open, onClose, onEndpointsChanged, onClearSessions, preferences, onPreferencesChange, appearance, onAppearanceChange, notify }: SettingsModalProps) {
  // This dialog is the app's only `showModal()` window, and the browser paints
  // that in the top layer -- above any z-index on the page. Claiming the host
  // while it is open is what puts the notification stack on top of it rather
  // than behind it, where an error about adding a model would be invisible at
  // the moment it is most relevant.
  //
  // The dialog element is state rather than a ref because the claim has to be
  // able to *react* to the element appearing. A ref is filled after the first
  // render and never triggers one, so an effect depending on `ref.current` would
  // run once with `null` and then never again. A callback ref keeps both the
  // `showModal`/`close` effect and the claim fed from the same value.
  const [dialog, setDialog] = useState<HTMLDialogElement | null>(null);
  useClaimNotificationHost(open, dialog);
  const nameRef = useRef<HTMLInputElement>(null);
  // The theme actually on screen, so each accent swatch previews the colour it
  // takes in the theme the user is looking at rather than in the other one.
  const resolvedTheme = resolveTheme(appearance.theme);
  const localSettingsSaveQueue = useRef(Promise.resolve());
  const [endpoints, setEndpoints] = useState<Endpoint[]>([]);
  const [expandedProviders, setExpandedProviders] = useState<string[]>([]);
  const [providerModelQueries, setProviderModelQueries] = useState<Record<string, string>>({});
  const [localModels, setLocalModels] = useState<LocalModel[]>([]);
  /**
   * How many downloads the backend holds, so the sidebar can offer Downloads
   * while there is something to look at.
   *
   * Only a count is kept. The full list changes several times a second, and
   * re-rendering this whole dialog on every progress tick would make the settings
   * window the slowest thing in the app.
   */
  const [transferCount, setTransferCount] = useState(0);
  /** The engine a dragged GGUF file is hovering over, while it is over one. */
  const [fileDropEngine, setFileDropEngine] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    let stop: (() => void) | undefined;
    const apply = (jobs: DownloadJob[]) => {
      setTransferCount((previous) => (previous === jobs.length ? previous : jobs.length));
    };
    void invoke<DownloadJob[]>("downloads_jobs")
      .then((jobs) => {
        if (live) apply(jobs);
      })
      .catch(() => undefined);
    void listen<DownloadJob[]>("downloads-state", (event) => apply(event.payload)).then(
      (unlisten) => {
        if (live) stop = unlisten;
        else unlisten();
      },
    );
    return () => {
      live = false;
      stop?.();
    };
  }, []);
  /**
   * Which engine kind the Local page is showing.
   *
   * Held here rather than in the tab strip so it survives a trip to a model's
   * settings page and back -- coming back to llama.cpp because the page happened
   * to remount would lose the user's place.
   */
  const [localCategory, setLocalCategory] = useState<LocalCategory>("llama");
  /**
   * Which engine groups are open on the Local models page.
   *
   * Engine ids rather than group indices, so a group's expansion survives a
   * reload: the same engine is the same id whether or not a model has been added
   * to it since, which an index would not be.
   */
  const [expandedEngines, setExpandedEngines] = useState<string[]>([]);
  /**
   * The model being dragged, and the one picked up by pressing its handle.
   *
   * Two pieces of state for one idea, because there are two ways to start a
   * move: a drag has the pointer held down and ends on release, while a pick-up
   * has to survive until a destination is chosen. Both feed the same drop, so a
   * destination never has to know which gesture put a model in flight.
   */
  const [draggingModel, setDraggingModel] = useState<string | null>(null);
  const [pickedModel, setPickedModel] = useState<{ id: string; name: string } | null>(null);
  /** The group under the pointer during a drag, for the drop highlight. */
  const [dropEngine, setDropEngine] = useState<string | null>(null);
  /** Set when a drag ends, so the click that follows it does not also arm the model. */
  const suppressClick = useRef(false);
  /**
   * The engines the group headers can name, and which of them exist.
   *
   * It stays in step with the group list because both are refreshed by
   * `updateLocalModelEngine`; `refreshEngines` handles the Engines page changing
   * names from under it.
   *
   * `installed` is carried because this is also what decides which groups the
   * Local models page draws -- the command returns the whole catalog, so an
   * unfiltered list here would offer drop targets for thirty releases nobody has
   * downloaded.
   */
  const [engineNames, setEngineNames] = useState<Pick<InstalledEngine, "id" | "version" | "source" | "installed">[]>([]);
  const [generalSettings, setGeneralSettings] = useState<GeneralSettings>(DEFAULT_GENERAL);
  const [confirmClear, setConfirmClear] = useState(false);
  const [clearingSessions, setClearingSessions] = useState(false);
  const [instructionExampleIndex, setInstructionExampleIndex] = useState(0);
  const [page, setPage] = useState<SettingsPage>({ view: "providers" });
  /**
   * Assist's record and Taste's record, read by the *modal* rather than by their pages.
   *
   * The dialog outlives the page: it is mounted while open and the page is only the
   * selected section, so a read started here runs while the user is still clicking around
   * in another section -- before the Assist or Taste click that would otherwise be the
   * first moment the read could begin, and that click is exactly when they are waiting.
   * Mounting the hook in the page instead meant the "Reading settings…" placeholder was
   * on screen for the whole read every single visit.
   *
   * Both are always enabled: `enabled` is not about whether the feature is on, it is
   * about whether the hook is subscribed at all, and "the user may look at this page" is
   * the answer here.
   */
  const assistStore = useAssist(open);
  const tasteStore = useTaste(open);
  /**
   * The row a search result landed on, highlighted for a moment.
   *
   * Search exists because a settings window with three groups is still one where a user
   * has to guess which page owns a row. Opening the right page is half the answer; the
   * other half is showing *where* on the page, so the match fades in and out rather than
   * leaving them to scan. `null` is ordinary navigation.
   */
  const [highlight, setHighlight] = useState<{ anchor: string; token: number } | null>(null);
  const [draft, setDraft] = useState<EndpointDraft>(blankEndpoint);
  const [showKey, setShowKey] = useState(false);
  const [loading, setLoading] = useState(false);
  const [testingId, setTestingId] = useState<string | null>(null);
  // `formError` is the one message that stays put: the provider form's own
  // validation, which belongs next to the fields and focuses the offending input.
  // Everything that reports an *outcome* -- a save, a test, a model added or
  // removed -- goes through `notify` and appears in the corner stack instead,
  // where it is visible from every settings page rather than only the one that
  // raised it, and where it does not scroll out of view.
  const [formError, setFormError] = useState("");

  // Preferences is its own page, but it shares the provider list already
  // loaded here so the model field renders the same options as the chat picker.
  // `endpoint.models` holds the available ones, so nothing is filtered here.
  const providerModelGroups = endpoints
    .filter((endpoint) => endpoint.enabled)
    .map((endpoint) => ({ endpoint, models: endpoint.models }))
    .filter((group) => group.models.length > 0);

  /**
   * Every row a search can find, grouped by the page that owns it.
   *
   * Kept in `settings/settingsIndex.ts` rather than inline because it is a list that has to
   * be kept in step with the pages by hand either way -- the pages are JSX, not data -- and
   * a list next to the JSX is one nobody can read while editing them. It is built on each
   * render and is cheap: plain objects, memoised nothing, and no read of any kind.
   */
  const searchEntries: SettingsEntry[] = buildSettingsIndex();

  /**
   * Navigates to a search result, then marks the row it names.
   *
   * The token is what makes landing on the same row twice still restart the mark: a
   * boolean would already be true, and a row that did not flash the second time reads as a
   * click that did nothing.
   */
  const openSearchResult = (entry: SettingsEntry) => {
    setPage(pageForEntry(entry));
    setHighlight({ anchor: entry.anchor, token: Date.now() });
  };

  // The mark is drawn by the page, not here: `FlashHighlight` listens for this event and
  // sets the attribute that the stylesheet animates. Announcing it after paint is what
  // lets the page that was just selected find its own row; before paint there would be
  // nothing to mark yet.
  useEffect(() => {
    if (!highlight) return;
    let cleared: number | undefined;
    const frame = requestAnimationFrame(() => {
      const anchor = highlight.anchor;
      document
        .querySelector(`[${ANCHOR_ATTRIBUTE}="${CSS.escape(anchor)}"]`)
        ?.scrollIntoView({ block: "center", behavior: "smooth" });
      window.dispatchEvent(new CustomEvent(SETTINGS_HIGHLIGHT_EVENT, { detail: { anchor } }));
      // Cleared one beat after the mark's own lifetime, so a repeat can always start a
      // fresh animation rather than arriving on a state that never changed.
      cleared = window.setTimeout(() => setHighlight(null), HIGHLIGHT_MS + 100);
    });
    return () => {
      cancelAnimationFrame(frame);
      window.clearTimeout(cleared);
    };
  }, [highlight]);

  /**
   * Local models filed under the engine that will run them.
   *
   * Built from `model.engine_id`, which the backend resolves as the model's own
   * engine -- the same answer `LocalModelManager::load` uses to pick the
   * executable. That is the whole reason this grouping can be trusted: it is
   * not a second opinion about which engine a model uses, it is that one
   * opinion, carried over.
   *
   * A model with no engine gets its own group rather than being hidden. No
   * engine is a real state -- such a model cannot be loaded at all -- and
   * dropping those rows would make an unchosen model look absent rather than
   * unrunnable. Ordered last, since that group is the one needing attention
   * rather than the one being browsed.
   *
   * **Every installed engine is seeded, not just the ones with models.** A group
   * has to exist because the engine exists: an engine with nothing on it is
   * precisely where a model needs to be draggable *to*, and a destination that
   * only appears once it is occupied cannot be reached. Installed only -- the
   * list carries the whole catalog, and offering a release that is not on disk
   * would be offering a move that fails at load.
   */
  const localModelGroups = (() => {
    const byEngine = new Map<string, LocalModel[]>();
    for (const engine of engineNames) {
      if (engine.installed) byEngine.set(engine.id, []);
    }
    for (const model of localModels) {
      // `""` rather than a null key: React needs a string, and the empty id is
      // what "no engine" already means everywhere else on this page.
      const key = model.engine_id ?? "";
      const existing = byEngine.get(key);
      if (existing) existing.push(model);
      else byEngine.set(key, [model]);
    }
    return [...byEngine.entries()]
      .map(([id, models]) => {
        const engine = engineNames.find((entry) => entry.id === id);
        return {
          id,
          // An id with no installed engine behind it is a choice pointing at
          // something since removed, which is not the same as no choice -- the
          // empty id -- and telling them apart is what stops a vanished engine
          // reading as merely unassigned.
          label: id === "" ? "No engine" : engine ? engineLabel(engine) : `${id} · not installed`,
          models,
        };
      })
      .sort((left, right) => (left.id === "" ? 1 : right.id === "" ? -1 : left.label.localeCompare(right.label)));
  })();

  // Capabilities for every listed model, one request per provider rather than
  // one per model. Absent until the provider has been tested, which is why a row
  // can show no badges at all.
  const [capabilitiesByModel, setCapabilitiesByModel] = useState<Record<string, ModelCapabilities>>({});
  const modelsByEndpoint = Object.fromEntries(endpoints.map((endpoint) => [endpoint.id, endpoint.models]));
  const endpointSignature = endpoints.map((endpoint) => endpoint.id).join(",");
  useEffect(() => {
    let active = true;
    void loadManyCapabilities(
      endpoints.map((endpoint) => endpoint.id),
      modelsByEndpoint,
    ).then((loaded) => {
      if (active) setCapabilitiesByModel(loaded);
    });
    return () => { active = false; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [endpointSignature, endpoints.map((endpoint) => endpoint.models.join("|")).join("~")]);

  useEffect(() => {
    if (!dialog) return;
    if (open && !dialog.open) dialog.showModal();
    if (!open && dialog.open) dialog.close();
  }, [dialog, open]);

  useEffect(() => {
    if (!open) return;
    let mounted = true;
    invoke<Partial<GeneralSettings> | null>("database_get_setting", { key: "app.general" })
      .then((saved) => {
        if (!mounted) return;
        if (saved) setGeneralSettings(withGeneralDefaults(saved));
        else {
          const legacy = localStorage.getItem("projectz.general-settings");
          const migrated = withGeneralDefaults(legacy ? JSON.parse(legacy) as Partial<GeneralSettings> : null);
          setGeneralSettings(migrated);
          if (legacy) void invoke("database_set_setting", { key: "app.general", value: migrated });
        }
      })
      .catch(() => { if (mounted) notify("error", "Could not load saved preferences."); });
    Promise.all([
      invoke<Endpoint[]>("ai_list_endpoints"),
      invoke<LocalModel[]>("local_models_list"),
    ])
      .then(([items, models]) => {
        if (!mounted) return;
        setEndpoints(items);
        setLocalModels(models);
      })
      .catch(() => { if (mounted) notify("error", "Could not load saved settings."); });
    // The engine list, for the group headers. A failure here is deliberately
    // silent: it leaves a header reading "No engine", and the Engines page --
    // which reports its own failures properly -- is where that gets fixed.
    void invoke<InstalledEngine[]>("local_engines_list", { refresh: false })
      .then((items) => { if (mounted) setEngineNames(items); })
      .catch(() => undefined);
    return () => { mounted = false; };
  }, [open, notify]);

  /**
   * Re-read the engine list after the Engines page changes it.
   *
   * An install or a removal there changes what the group headers can name, and
   * a Local models page still showing the old list would label a group with an
   * engine that is no longer there -- two views disagreeing about one thing,
   * which is the failure the whole single-source arrangement exists to prevent.
   */
  const refreshEngines = async () => {
    try {
      setEngineNames(await invoke<InstalledEngine[]>("local_engines_list", { refresh: false }));
    } catch {
      // As above: the page above reports its own failures.
    }
  };

  const openAdd = () => {
    setDraft(blankEndpoint());
    setPage({ view: "form", endpoint: null });
    setFormError("");
  };

  const openEdit = (endpoint: Endpoint) => {
    setDraft({ ...endpoint, api_key: "" });
    setPage({ view: "form", endpoint });
    setFormError("");
  };

  const refreshEndpoints = async () => {
    const items = await invoke<Endpoint[]>("ai_list_endpoints");
    setEndpoints(items);
    onEndpointsChanged();
    return items;
  };

  const setProviderModelEnabled = (endpoint: Endpoint, model: string, enabled: boolean) => {
    void invoke("ai_set_model_enabled", { endpointId: endpoint.id, model, enabled })
      .then(refreshEndpoints)
      .catch((reason: unknown) => notify("error", `Model availability could not be saved: ${String(reason)}`));
  };

  const setProviderModelsEnabled = (endpoint: Endpoint, enabled: boolean) => {
    // Every known model, not just the enabled ones, and only the ones whose state
    // actually changes. A provider's catalogue now arrives switched off, so
    // "Enable all" has to reach the disabled models -- they are exactly what the
    // click is asking for -- while models already in the wanted state are left
    // alone so a flip touches only what it must.
    const targets = [...endpoint.models, ...endpoint.disabled_models]
      .filter((model) => endpoint.disabled_models.includes(model) === enabled);
    // Sequential rather than `all`, because each write is its own row on the same
    // connection: a gateway reporting several hundred models would otherwise open
    // several hundred concurrent writes against one database for one click.
    const work = targets.reduce(
      (chain, model) => chain.then(() => invoke("ai_set_model_enabled", { endpointId: endpoint.id, model, enabled })),
      Promise.resolve(),
    );
    void work
      .then(refreshEndpoints)
      .catch((reason: unknown) => notify("error", `Model availability could not be saved: ${String(reason)}`));
  };

  const saveEndpoint = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!draft.name.trim() || !draft.base_url.trim()) {
      setFormError("Enter a provider name and base URL.");
      if (!draft.name.trim()) nameRef.current?.focus();
      return;
    }
    try {
      setLoading(true);
      setFormError("");
      const editing = page.view === "form" && page.endpoint;
      const command = editing ? "ai_update_endpoint" : "ai_add_endpoint";
      await invoke(command, { endpoint: { ...draft, name: draft.name.trim(), base_url: draft.base_url.trim(), api_key: draft.api_key.trim() } });
      await refreshEndpoints();
      notify("success", editing ? `${draft.name} updated` : `${draft.name} added`);
      setPage({ view: "providers" });
    } catch (reason) {
      notify("error", `Provider could not be saved: ${String(reason)}`);
    } finally {
      setLoading(false);
    }
  };

  const testEndpoint = async (endpoint: Endpoint | EndpointDraft) => {
    // Keyed by provider id so a second test of the same provider replaces the
    // first row rather than stacking a second copy of it.
    const key = `settings:test:${endpoint.id}`;
    try {
      setTestingId(endpoint.id);
      notify("loading", `Connecting to ${endpoint.name}…`, key);
      const models = await invoke<string[]>("ai_test_endpoint", {
        endpoint: { ...endpoint, api_key: "api_key" in endpoint ? endpoint.api_key : "" },
      });
      if ("has_api_key" in endpoint) {
        await invoke("ai_update_endpoint", { endpoint: { ...endpoint, models } });
        await refreshEndpoints();
      }
      notify("success", `${endpoint.name} connected · ${models.length} models found`, key);
      if (page.view === "form") setDraft((current) => ({ ...current, models }));
    } catch (reason) {
      notify("error", `Test failed: ${String(reason)}`, key);
    } finally {
      setTestingId(null);
    }
  };

  useEffect(() => {
    const timer = window.setInterval(() => setInstructionExampleIndex((index) => (index + 1) % INSTRUCTION_EXAMPLES.length), 7000);
    return () => window.clearInterval(timer);
  }, []);

  /**
   * Persists one model's runtime settings.
   *
   * The page owns the draft, so the modal is only the thing that writes it. The
   * queue is what makes that safe: a slider emits a change per step, and two
   * overlapping writes could land out of order and leave the stored value behind
   * the one on screen. Serialising them means the last write wins by
   * construction.
   *
   * No success message. The control already shows the value that was chosen, so
   * a line announcing the app did what was asked states nothing new -- and the
   * space it occupied is where the error belongs. The error itself now goes to
   * the notification stack, because a control at the bottom of a scrolled page
   * is exactly where an inline line would be least likely to be seen.
   */
  const saveLocalRuntimeSettings = (modelId: string, settings: LocalRuntimeSettings) => {
    localSettingsSaveQueue.current = localSettingsSaveQueue.current
      .catch(() => undefined)
      .then(() => invoke("local_runtime_settings_save", { modelId, settings }))
      // Mapped to void so the queue ref keeps a `Promise<void>` type; the chain
      // must not widen to `Promise<unknown>` or every later `.then` is untyped.
      .then(() => undefined)
      .catch((reason: unknown) => notify("error", `Settings could not be saved: ${String(reason)}`, `settings:runtime:${modelId}`));
  };

  const updateGeneralSettings = (next: GeneralSettings) => {
    setGeneralSettings(next);
    void invoke("database_set_setting", { key: "app.general", value: next }).catch((reason) => notify("error", `Preferences could not be saved: ${String(reason)}`, "settings:general"));
  };

  // Two steps on purpose: the row button only arms the confirmation, and the dialog
  // inside it is the thing that actually deletes. Closing resets the armed state.
  const clearSessions = async () => {
    setClearingSessions(true);
    try {
      const removed = await onClearSessions();
      setConfirmClear(false);
      // "Nothing to delete" is a success, not a failure: the delete did what was
      // asked, which was to leave no sessions behind. It is reported at all only
      // because a confirmation dialog that closes in silence reads as a dismissal.
      notify("success", removed === 0 ? "No sessions to delete" : `${removed} session${removed === 1 ? "" : "s"} deleted`);
    } catch (reason) {
      setConfirmClear(false);
      notify("error", `Sessions could not be deleted: ${String(reason)}`);
    } finally {
      setClearingSessions(false);
    }
  };

  /**
   * Re-reads the local model list from the backend.
   *
   * Two writers change it without this component doing the writing -- `add` above,
   * and a download that registers what it installed -- so the list is refetched
   * rather than patched. `inspect` fills in quantisation, context window and
   * presence, which a locally appended row would not have.
   */
  const refreshLocalModels = async () => {
    try {
      setLocalModels(await invoke<LocalModel[]>("local_models_list"));
    } catch {
      // A failed refresh leaves the list as it was; the action that caused it has
      // already reported its own outcome.
    }
  };

  /** The engine group under a client-space point, if the pointer is over one. */
  const engineAt = (x: number, y: number): string | null => {
    const element = document.elementFromPoint(x, y);
    return element?.closest<HTMLElement>("[data-engine-group]")?.dataset.engineGroup ?? null;
  };

  /**
   * Adds GGUF files dropped from Explorer, and puts them on the engine they were
   * dropped on.
   *
   * One loop rather than `Promise.all`: each add is a database write plus a read
   * of the file's header, and a handful of models arriving in parallel contend
   * for the same lock for no gain.
   */
  const addDroppedModels = async (paths: string[], engineId: string | null) => {
    let added = 0;
    for (const path of paths) {
      try {
        const model = await invoke<LocalModel>("local_models_add", { path });
        if (engineId) {
          await invoke("local_model_engine_set", { modelId: model.id, engineId });
        }
        added += 1;
      } catch (reason) {
        notify("error", `Could not add ${path}: ${String(reason)}`);
      }
    }
    if (added === 0) return;
    await refreshLocalModels();
    onEndpointsChanged();
    notify("success", added === 1 ? "1 model added" : `${added} models added`);
  };

  /**
   * Files dragged in from Explorer, while the llama.cpp page is the one showing.
   *
   * Tauri consumes the native drag to implement its own file drop, so the paths
   * arrive through this event rather than a DOM `drop` -- the same route the
   * composer uses to attach a file. Listening only on this page means a file
   * dropped anywhere else cannot quietly register a model.
   */
  useEffect(() => {
    if (page.view !== "local" || localCategory !== "llama") return;
    let live = true;
    let stop: (() => void) | undefined;
    const scale = () => window.devicePixelRatio || 1;
    void getCurrentWebview()
      .onDragDropEvent((event) => {
        const payload = event.payload;
        if (payload.type === "drop") {
          setFileDropEngine(null);
          const paths = payload.paths.filter((path) => path.toLowerCase().endsWith(".gguf"));
          if (paths.length === 0) return;
          void addDroppedModels(
            paths,
            engineAt(payload.position.x / scale(), payload.position.y / scale()),
          );
          return;
        }
        if (payload.type === "leave") {
          setFileDropEngine(null);
          return;
        }
        setFileDropEngine(engineAt(payload.position.x / scale(), payload.position.y / scale()));
      })
      .then((unlisten) => {
        if (live) stop = unlisten;
        else unlisten();
      });
    return () => {
      live = false;
      stop?.();
    };
    // `addDroppedModels` and `engineAt` read state through their closures and are
    // rebuilt with it; re-binding on the two things that decide whether a drop is
    // welcome is enough.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [page.view, localCategory]);

  const addLocalModel = async () => {
    const selected = await openFile({
      multiple: false,
      filters: [{ name: "GGUF model", extensions: ["gguf"] }],
    });
    if (typeof selected !== "string") return;
    try {
      const added = await invoke<LocalModel>("local_models_add", { path: selected });
      // Re-read rather than appending the returned row. The row that comes back
      // from `add` has not been through `inspect`, so it carries no quantisation,
      // no context window and no presence check -- exactly the three facts the
      // list draws. One extra read on a rare action, for a row that is correct
      // rather than merely present.
      setLocalModels(await invoke<LocalModel[]>("local_models_list"));
      onEndpointsChanged();
      // The added model comes back from `local_models_list`, not from `add`, so
      // the name to confirm with is the one the list actually shows.
      notify("success", `${added.name} added`);
    } catch (reason) {
      // The common failure here is not a network or disk error but the user
      // picking a file that is already registered, which arrives as a message
      // naming the duplicate. It is reported rather than swallowed, because
      // "nothing happened" is the worst possible answer to a deliberate action.
      notify("error", `Model could not be added: ${String(reason)}`);
    }
  };

  const removeLocalModel = async (model: LocalModel) => {
    try {
      await invoke("local_models_remove", { id: model.id });
      setLocalModels((current) => current.filter((item) => item.id !== model.id));
      onEndpointsChanged();
      notify("success", `${model.name} removed`);
    } catch (reason) {
      notify("error", `Model could not be removed: ${String(reason)}`);
    }
  };

  /**
   * Re-reads the models after an engine change.
   *
   * A refetch rather than a local edit, because `engine_id` on a row is
   * *resolved* by the backend rather than stored, and writing it here would mean
   * this page reproducing the pin-then-default order. That is the one fact in
   * this file that must not have a second copy.
   */
  const updateLocalModelEngine = async () => {
    setLocalModels(await invoke<LocalModel[]>("local_models_list"));
  };

  /** The model in flight, whichever gesture put it there. */
  const movingModelId = draggingModel ?? pickedModel?.id ?? null;

  /**
   * True when a model is in flight and this group is somewhere it could land.
   *
   * The `""` group -- models with no engine at all -- is never a destination:
   * the backend has no command to unset an engine, and a drop that silently did
   * nothing would be worse than a group that visibly refuses. A model already
   * filed under the group is refused too, since that move is a no-op.
   */
  const canDropOn = (groupId: string) => {
    if (!movingModelId || !groupId) return false;
    return (localModels.find((model) => model.id === movingModelId)?.engine_id ?? "") !== groupId;
  };

  /**
   * The engine group under a viewport point, when it is one this model may move
   * to.
   *
   * Hit-tested from the document rather than collected from each group's own
   * `dragenter`, because this drag runs on pointer events: the pointer is
   * captured by the row it started on, so the groups never receive an event of
   * their own to react to. `data-engine-group` on each header is what this looks
   * for.
   */
  const dropTargetAt = (x: number, y: number, modelId: string): string | null => {
    const groupId = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-engine-group]")?.dataset.engineGroup ?? null;
    if (!groupId) return null;
    return (localModels.find((model) => model.id === modelId)?.engine_id ?? "") === groupId ? null : groupId;
  };

  /**
   * Moves a model onto an engine, and re-files it.
   *
   * The in-flight state is cleared first, so a failed move still ends the
   * gesture: a model left stuck to the pointer with an error in the corner is
   * the one outcome worse than the failure itself.
   */
  const moveModelToEngine = async (modelId: string, engineId: string) => {
    const name = localModels.find((model) => model.id === modelId)?.name ?? "Model";
    setDraggingModel(null);
    setPickedModel(null);
    setDropEngine(null);
    try {
      await invoke("local_model_engine_set", { modelId, engineId });
      await updateLocalModelEngine();
      notify("success", `${name} moved to ${engineLabel(engineNames.find((engine) => engine.id === engineId))}`);
    } catch (reason) {
      notify("error", `Engine could not be changed: ${String(reason)}`);
    }
  };

  const formatSize = (bytes: number) => {
    const gib = bytes / (1024 ** 3);
    return gib >= 1 ? `${gib.toFixed(1)} GB` : `${(bytes / (1024 ** 2)).toFixed(0)} MB`;
  };

  const removeEndpoint = async (endpoint: Endpoint) => {
    try {
      await invoke("ai_remove_endpoint", { id: endpoint.id });
      setEndpoints((current) => current.filter((item) => item.id !== endpoint.id));
      onEndpointsChanged();
      notify("success", `${endpoint.name} removed`);
    } catch (reason) {
      notify("error", `Provider could not be removed: ${String(reason)}`);
    }
  };

  // Where each drilled-in page returns to, and what the back control is called.
  // The provider form and the skill form are the same shape -- a whole page
  // standing in for a list -- so the header treats them the same way.
  const isSubPage = page.view === "form" || page.view === "model" || page.view === "skill" || page.view === "download" || page.view === "ttsSettings" || page.view === "ttsVoice" || page.view === "sttModel";
  const backTarget: SettingsPage =
    page.view === "model" || page.view === "download" || page.view === "ttsSettings" || page.view === "ttsVoice" || page.view === "sttModel" ? { view: "local" } : page.view === "skill" ? { view: "skills" } : { view: "providers" };
  const backLabel =
    page.view === "model" ? "Back to local models" : page.view === "download" ? "Back to local engines" : page.view === "ttsVoice" ? "Back to speech synthesis" : page.view === "ttsSettings" ? "Back to speech synthesis" : page.view === "sttModel" ? "Back to speech recognition" : page.view === "skill" ? "Back to skills" : "Back to providers";
  // Held outside the JSX so the save callback can compare against it without
  // relying on narrowing that does not survive into a closure.
  const editingSkill = page.view === "skill" ? page.slug : null;

  return (
    <dialog
      ref={setDialog}
      aria-labelledby="settings-title"
      onClose={onClose}
      // Escape cancels a model that is mid-move rather than closing the window.
      // Intercepting the dialog's own cancel event is the only way to stop it:
      // the browser closes the dialog itself, and that is not a keydown this
      // component ever sees.
      onCancel={(event) => { if (pickedModel) { event.preventDefault(); setPickedModel(null); } }}
      onClick={(event) => { if (event.target === dialog) onClose(); }}
      className="m-0 h-dvh w-dvw max-h-none max-w-none overflow-hidden border-0 bg-transparent p-0 text-[var(--text)] backdrop:bg-black/65"
    >
      <div className="mx-auto flex h-full w-full max-w-[1000px] flex-col overflow-hidden border-x border-[var(--line)] bg-[var(--page)] shadow-2xl sm:my-[6vh] sm:h-[88vh] sm:rounded-lg sm:border">
        {/* The header is separated from the page below it by a hairline, the same
            way the sidebar is separated from the content. Without it the search
            field floated over whatever the page drew first; the page's own title
            band then drew the second rule, a few pixels lower. */}
        <header className="flex min-h-[60px] items-center justify-between border-b border-[var(--line)] px-4 sm:px-6">
          <div className="flex min-w-0 flex-1 items-center gap-2.5">
            {isSubPage && <button type="button" onClick={() => setPage(backTarget)} className="grid size-8 place-items-center rounded-md text-[var(--muted)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]" aria-label={backLabel} aria-labelledby={undefined}><ArrowLeft size={16} /></button>}
            {/* The search field takes the place the static "Settings / General" label
                used to hold. A window with eleven sections is one where a reader has to
                guess which page owns a row, and a title that only repeated the page they
                already clicked answered a question nobody asked. The page's own name is
                on the page, under its heading. */}
            <SettingsSearch entries={searchEntries} onOpen={openSearchResult} />
          </div>
          <button type="button" onClick={onClose} className="grid size-8 place-items-center rounded-md text-[var(--muted)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]" aria-label="Close settings"><X size={17} /></button>
        </header>

        <div className="flex min-h-0 flex-1 flex-col sm:flex-row">
          {/* No bottom hairline in the stacked layout either: the title band already draws
              one directly under it, and two rules a few pixels apart is the duplicate. */}
          <nav aria-label="Settings sections" className="flex shrink-0 gap-1 overflow-x-auto bg-[var(--rail)] px-2.5 py-2 sm:min-h-0 sm:w-[200px] sm:flex-col sm:gap-0.5 sm:overflow-y-auto sm:border-r sm:border-[var(--line)] sm:py-3">
            <p className="hidden px-2.5 pb-1.5 pt-1 text-[11px] font-medium text-[var(--quiet)] sm:block">Application</p>
            <button type="button" data-settings-nav="general" aria-label="General" onClick={() => setPage({ view: "general" })} className={`flex min-h-9 items-center gap-2.5 rounded-md px-2.5 text-left text-[13px] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)] ${page.view === "general" ? "bg-[color-mix(in_srgb,var(--accent)_13%,transparent)] text-[var(--accent)]" : "text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]"}`}><SlidersHorizontal size={16} />General</button>
            <button type="button" data-settings-nav="appearance" aria-label="Appearance" onClick={() => setPage({ view: "appearance" })} className={`flex min-h-9 items-center gap-2.5 rounded-md px-2.5 text-left text-[13px] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)] ${page.view === "appearance" ? "bg-[color-mix(in_srgb,var(--accent)_13%,transparent)] text-[var(--accent)]" : "text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]"}`}><Palette size={16} />Appearance</button>
            <button type="button" data-settings-nav="preferences" aria-label="Preferences" onClick={() => setPage({ view: "preferences" })} className={`flex min-h-9 items-center gap-2.5 rounded-md px-2.5 text-left text-[13px] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)] ${page.view === "preferences" ? "bg-[color-mix(in_srgb,var(--accent)_13%,transparent)] text-[var(--accent)]" : "text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]"}`}><Sparkles size={16} />Preferences</button>
            {/* The assistant's own surfaces: what it does (Assist), what it can load
                (Skills) and what it can reach (MCP). They were in Application, where a
                reader looking for "where do I change the browser" found them mixed in
                with the theme picker. */}
            <p className="hidden px-2.5 pb-1.5 pt-5 text-[11px] font-medium text-[var(--quiet)] sm:block">Assistant</p>
            <button type="button" data-settings-nav="assist" aria-label="Assist" onClick={() => setPage({ view: "assist" })} className={`flex min-h-9 items-center gap-2.5 rounded-md px-2.5 text-left text-[13px] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)] ${page.view === "assist" ? "bg-[color-mix(in_srgb,var(--accent)_13%,transparent)] text-[var(--accent)]" : "text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]"}`}><Lightbulb size={16} />Assist</button>
            <button type="button" data-settings-nav="taste" aria-label="Taste" onClick={() => setPage({ view: "taste" })} className={`flex min-h-9 items-center gap-2.5 rounded-md px-2.5 text-left text-[13px] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)] ${page.view === "taste" ? "bg-[color-mix(in_srgb,var(--accent)_13%,transparent)] text-[var(--accent)]" : "text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]"}`}><Brain size={16} />Taste</button>
            <button type="button" data-settings-nav="skills" aria-label="Skills" onClick={() => setPage({ view: "skills" })} className={`flex min-h-9 items-center gap-2.5 rounded-md px-2.5 text-left text-[13px] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)] ${page.view === "skills" || page.view === "skill" ? "bg-[color-mix(in_srgb,var(--accent)_13%,transparent)] text-[var(--accent)]" : "text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]"}`}><BookMarked size={16} />Skills</button>
            <button type="button" data-settings-nav="mcp" aria-label="MCP servers" onClick={() => setPage({ view: "mcp" })} className={`flex min-h-9 items-center gap-2.5 rounded-md px-2.5 text-left text-[13px] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)] ${page.view === "mcp" ? "bg-[color-mix(in_srgb,var(--accent)_13%,transparent)] text-[var(--accent)]" : "text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]"}`}><Plug size={16} />MCP</button>
            <p className="hidden px-2.5 pb-1.5 pt-5 text-[11px] font-medium text-[var(--quiet)] sm:block">Models</p>
            <button type="button" data-settings-nav="providers" aria-label="Providers" onClick={() => setPage({ view: "providers" })} className={`flex min-h-9 items-center gap-2.5 rounded-md px-2.5 text-left text-[13px] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)] ${page.view === "providers" ? "bg-[color-mix(in_srgb,var(--accent)_13%,transparent)] text-[var(--accent)]" : "text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]"}`}><Server size={16} />Providers</button>
            <button type="button" data-settings-nav="local" aria-label="Local models" onClick={() => setPage({ view: "local" })} className={`flex min-h-9 items-center gap-2.5 rounded-md px-2.5 text-left text-[13px] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)] ${page.view === "local" || page.view === "model" ? "bg-[color-mix(in_srgb,var(--accent)_13%,transparent)] text-[var(--accent)]" : "text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]"}`}><Cpu size={16} />Local</button>
            <button type="button" data-settings-nav="engines" aria-label="Engines" onClick={() => setPage({ view: "engines" })} className={`flex min-h-9 items-center gap-2.5 rounded-md px-2.5 text-left text-[13px] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)] ${page.view === "engines" ? "bg-[color-mix(in_srgb,var(--accent)_13%,transparent)] text-[var(--accent)]" : "text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]"}`}><Zap size={16} />Engines</button>
            {/* Only while something is transferring, or has this session: a permanent
                "Downloads" entry for a list that is usually empty is a page nobody
                needs and a row that says nothing. */}
            {transferCount > 0 && (
              <button type="button" data-settings-nav="downloads" aria-label="Downloads" onClick={() => setPage({ view: "downloads" })} className={`flex min-h-9 items-center gap-2.5 rounded-md px-2.5 text-left text-[13px] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)] ${page.view === "downloads" ? "bg-[color-mix(in_srgb,var(--accent)_13%,transparent)] text-[var(--accent)]" : "text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]"}`}>
                <Download size={16} />
                Downloads
                <span className="ml-auto text-[11px] tabular-nums text-[var(--quiet)]">{transferCount}</span>
              </button>
            )}
            {/* On its own, at the bottom: System is not a category, and standing
                apart says so. `mt-auto` pins it once the list fits; when the window
                is too short, the whole nav scrolls and this comes last. */}
            <div aria-hidden="true" className="hidden sm:mt-auto sm:block sm:h-px sm:bg-[var(--line)]" />
            <button type="button" data-settings-nav="system" aria-label="System" onClick={() => setPage({ view: "system" })} className={`flex min-h-9 shrink-0 items-center gap-2.5 rounded-md px-2.5 text-left text-[13px] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)] ${page.view === "system" ? "bg-[color-mix(in_srgb,var(--accent)_13%,transparent)] text-[var(--accent)]" : "text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]"}`}><Info size={16} />System</button>
          </nav>

          <div className="min-w-0 flex-1 overflow-y-auto">
            <div className="mx-auto max-w-[760px] px-5 py-6 sm:px-9 sm:py-8">
              {/* No `key`: a search result on the page already open must not remount the
                  scope, or the dim it is about to set is thrown away with the old node. */}
              <SettingsSearchScope>
              {page.view === "assist" ? <AssistSettingsPage notify={notify} store={assistStore} groups={providerModelGroups} localModels={localModels} /> : page.view === "taste" ? <TasteSettingsPage notify={notify} store={tasteStore} groups={providerModelGroups} localModels={localModels} /> : page.view === "providers" ? <>
                <SettingsPageTitle>
                  <h2 className="text-[17px] font-semibold tracking-tight"><FlashHighlight anchor="providers.list" />Providers</h2>
                  <button type="button" {...{ [ANCHOR_ATTRIBUTE]: "providers.add" }} onClick={openAdd} className="inline-flex min-h-9 items-center gap-1.5 rounded-lg border border-[var(--line)] bg-[var(--panel)] px-3 text-[13px] font-medium text-[var(--text)] transition-colors hover:border-[var(--line-strong)] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"><FlashHighlight anchor="providers.add" /><Plus size={15} />Add provider</button>
                </SettingsPageTitle>
                {endpoints.length === 0 ? <div className="px-4 py-7"><p className="text-sm text-[var(--muted)]">No providers connected.</p><p className="mt-1 text-xs text-[var(--quiet)]">Add an endpoint to discover models and use them in chat.</p></div> : <div className="divide-y divide-[var(--line)]">
                  {endpoints.map((endpoint) => {
                    const expanded = expandedProviders.includes(endpoint.id);
                    // Both lists are drawn, switched-off ones last. They used to
                    // be one list with a flag beside each row, which meant a
                    // switched-off model had to be carried through the read
                    // separately anyway -- and the fact it was the single source
                    // of the flag is what let the two disagree.
                    const allModels = [...endpoint.models, ...endpoint.disabled_models];
                    const enabledCount = endpoint.models.length;
                    return <article key={endpoint.id}>
                      <div className="flex min-h-[68px] items-center gap-2 px-4 py-3">
                        <ProviderIcon baseUrl={endpoint.base_url} name={endpoint.name} expanded={expanded} onToggle={() => setExpandedProviders((current) => expanded ? current.filter((id) => id !== endpoint.id) : [...current, endpoint.id])} />
                        <div className="min-w-0 flex-1"><h3 className="truncate text-sm font-medium">{endpoint.name}</h3><p className="truncate text-xs text-[var(--quiet)]">{endpoint.base_url} · {enabledCount} of {allModels.length} models enabled</p></div>
                        <button type="button" onClick={() => void testEndpoint(endpoint)} disabled={testingId === endpoint.id} className="grid size-9 shrink-0 place-items-center rounded-md text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--accent)] disabled:opacity-50" aria-label={`Test ${endpoint.name}`} title="Test connection and find models"><Zap size={16} className={testingId === endpoint.id ? "animate-pulse" : ""} /></button>
                        <button type="button" onClick={() => openEdit(endpoint)} className="grid size-9 shrink-0 place-items-center rounded-md text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]" aria-label={`Edit ${endpoint.name}`} title="Edit provider"><Pencil size={16} /></button>
                        <HoldToDelete onConfirm={() => void removeEndpoint(endpoint)} label={`Remove ${endpoint.name}`} title="Remove provider" className="grid size-9 shrink-0 place-items-center rounded-md text-[var(--quiet)] hover:bg-[var(--raised)] hover:text-[var(--danger)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]"><Trash2 size={16} /></HoldToDelete>
                      </div>
                      {expanded && <div className="mb-3 ml-[2.75rem] max-h-64 overflow-y-auto border-l border-[var(--line)] py-1 pl-3">
                        <div className="sticky top-0 z-10 flex flex-wrap items-center gap-x-4 gap-y-2 bg-[var(--page)] pb-2 pr-2">
                          <label className="flex min-h-9 min-w-0 flex-1 items-center gap-2 text-[var(--muted)] focus-within:text-[var(--text)]"><Search size={14} aria-hidden="true" /><span className="sr-only">Search {endpoint.name} models</span><input value={providerModelQueries[endpoint.id] ?? ""} onChange={(event) => setProviderModelQueries((current) => ({ ...current, [endpoint.id]: event.target.value }))} placeholder="Search models" className="min-w-0 flex-1 bg-transparent text-xs text-[var(--text)] outline-none placeholder:text-[var(--quiet)]" /></label>
                          <button type="button" disabled={allModels.length === 0} onClick={() => setProviderModelsEnabled(endpoint, enabledCount !== allModels.length)} className="min-h-8 shrink-0 rounded-md px-2 text-xs font-medium text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)] disabled:opacity-50">{allModels.length > 0 && enabledCount === allModels.length ? "Disable all" : "Enable all"}</button>
                        </div>
                        {allModels.length === 0 ? <p className="py-2 text-xs text-[var(--quiet)]">No models discovered. Test the provider to find models.</p> : <div className="divide-y divide-[var(--line)]">
                          {allModels.filter((model) => model.toLowerCase().includes((providerModelQueries[endpoint.id] ?? "").trim().toLowerCase())).map((model) => {
                            const enabled = !endpoint.disabled_models.includes(model);
                            return <button key={model} type="button" disabled={loading} aria-pressed={enabled} onClick={() => setProviderModelEnabled(endpoint, model, !enabled)} className="flex min-h-9 w-full items-center gap-3 py-1.5 pr-2 text-left text-xs hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)] disabled:opacity-50">
                              <span className="min-w-0 flex-1 truncate text-[var(--muted)]" title={model}>{model}</span><ModelBadges capabilities={capabilitiesByModel[`${endpoint.id}:${model}`]} /><span className={`shrink-0 rounded px-1.5 py-0.5 text-[10px] ${enabled ? "text-[var(--accent)]" : "text-[var(--quiet)]"}`}>{enabled ? "Enabled" : "Disabled"}</span>
                            </button>;
                          })}
                        </div>}
                      </div>}
                    </article>;
                  })}
                </div>}
              </> : page.view === "engines" ? <EnginesSettingsPage onChanged={() => { onEndpointsChanged(); void refreshEngines(); }} notify={notify} /> : page.view === "local" ? <>
                <SettingsPageTitle>
                  <h2 className="text-[17px] font-semibold tracking-tight"><FlashHighlight anchor="local.list" />{LOCAL_TITLES[localCategory]}</h2>
                  {/* llama.cpp is the only half with something to add. A Whisper
                      model and a Piper voice are placed by their own installers
                      rather than chosen from a file dialog, so a picker here would
                      offer a file the engine cannot load. */}
                  {localCategory === "llama" && <LocalAddMenu anchor="local.add" onDownload={() => setPage({ view: "download", kind: "llama" })} onAdd={() => void addLocalModel()} />}
                </SettingsPageTitle>
                {/* The three engines in one page rather than three sidebar entries:
                    they answer the same question -- "what is installed on this
                    machine" -- and a reader comparing them should not have to
                    leave the page to do it. */}
                <div className="mb-4"><LocalCategoryTabs value={localCategory} onChange={setLocalCategory} /></div>
                {localCategory === "llama" ? <>
                {/* Only while a model is armed. The move is a mode, and a mode
                    with no visible state is one the user cannot tell they are
                    in -- or get out of. */}
                {pickedModel && (
                  <p className="mb-3 flex items-center gap-2 rounded-md border border-[color-mix(in_srgb,var(--accent)_35%,var(--line))] bg-[color-mix(in_srgb,var(--accent)_8%,transparent)] px-3 py-2 text-[12px] text-[var(--text)]">
                    <MoveRight size={13} className="shrink-0 text-[var(--accent)]" />
                    Moving {pickedModel.name}. Choose an engine below, or press Escape to cancel.
                  </p>
                )}
                {localModels.length === 0 ? <div className="px-4 py-7"><p className="text-sm text-[var(--muted)]">No local models added.</p><p className="mt-1 text-xs text-[var(--quiet)]">Drop GGUF files here to add them, or choose one from the menu above.</p></div> : <div className="divide-y divide-[var(--line)]">
                  {/* Grouped by engine, the same shape the providers page uses.
                      The group is the engine, not the model: a model is run by one
                      engine, so grouping by it answers the question someone
                      actually has -- "which of these is using the fork, and what
                      else is on it" -- which a flat alphabetical list of models
                      cannot. `engine_id` arrives as the model's own engine from
                      the backend, so this never has to guess. */}
                  {localModelGroups.map((group) => <article key={group.id}
                    // The whole group is the target, not just its header. An open
                    // engine draws its models below the header, and dropping onto
                    // that list is the natural place to aim -- with the attribute
                    // only on the header, `closest` found nothing there and the
                    // drop was silently refused.
                    data-engine-group={group.id}
                  >
                    <div
                      // Marks the target for `dropTargetAt`, which hit-tests the
                      // document while a model is in flight. A collapsed group is
                      // still a target: opening it is what the drop is *for*, not
                      // a precondition of it.
                      data-engine-group={group.id}
                      className={`flex min-h-[68px] items-center gap-2 border-l-2 px-4 py-3 transition-colors ${dropEngine === group.id || fileDropEngine === group.id ? "border-[var(--accent)] bg-[color-mix(in_srgb,var(--accent)_10%,transparent)]" : "border-transparent"}`}
                    >
                      <button type="button" onClick={() => setExpandedEngines((current) => current.includes(group.id) ? current.filter((id) => id !== group.id) : [...current, group.id])} aria-expanded={expandedEngines.includes(group.id)} aria-label={`${expandedEngines.includes(group.id) ? "Collapse" : "Expand"} ${group.label}`} title={`${expandedEngines.includes(group.id) ? "Collapse" : "Expand"} ${group.label}`} className="grid size-9 shrink-0 place-items-center rounded-md text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"><Cpu size={16} /></button>
                      <div className="min-w-0 flex-1"><h3 className="truncate text-sm font-medium">{group.label}</h3><p className="truncate text-xs text-[var(--quiet)]">{group.models.length === 0 ? "No models" : group.models.length === 1 ? "1 model" : `${group.models.length} models`}{group.id ? "" : " · no engine chosen"}</p></div>
                      {/* How a move started with the handle is finished. A drag
                          needs a held pointer, so without this the feature does
                          not exist for anyone working from the keyboard. */}
                      {pickedModel && canDropOn(group.id) && (
                        <button type="button" onClick={() => void moveModelToEngine(pickedModel.id, group.id)} className="inline-flex min-h-8 shrink-0 items-center gap-1.5 rounded-md bg-[var(--accent)] px-2.5 text-[12px] font-medium text-[var(--accent-ink)] transition-opacity hover:opacity-90 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"><MoveRight size={13} />Move here</button>
                      )}
                      <span className="grid size-9 shrink-0 place-items-center text-[var(--quiet)]" aria-hidden="true">{expandedEngines.includes(group.id) ? <ChevronDown size={16} /> : <ChevronRight size={16} />}</span>
                    </div>
                    {expandedEngines.includes(group.id) && <div className="mb-3 ml-[2.75rem] border-l border-[var(--line)] py-1 pl-3">
                      {/* An engine with nothing on it is the whole point of
                          showing every engine, so an opened empty group says so
                          rather than rendering as a blank panel. */}
                      {group.models.length === 0 && <p className="py-2 text-xs text-[var(--quiet)]">Nothing on this engine yet. Drag a model here.</p>}
                      <div className="divide-y divide-[var(--line)]">
                        {group.models.map((model) => {
                          const picked = pickedModel?.id === model.id;
                          return <div
                            key={model.id}
                            title={fileLabel(model.path)}
                            // Pointer events, not HTML5 drag-and-drop. On Windows
                            // Tauri consumes the native drag to implement its own
                            // file drop, so `dragstart` never reaches the page --
                            // and switching that off would cost the chat its
                            // drop-a-file-to-attach.
                            //
                            // The move and release are listened for on `window`,
                            // not on this row. A fast drag leaves the row between
                            // two pointer events, and a row-local handler never
                            // hears the rest of the gesture: the model simply
                            // stayed where it was. Window listeners also remove the
                            // need for pointer capture, which would have swallowed
                            // the clicks on the buttons inside the row.
                            onPointerDown={(event) => {
                              if (event.button !== 0) return;
                              suppressClick.current = false;
                              const originX = event.clientX;
                              const originY = event.clientY;
                              let dragging = false;

                              const detach = () => {
                                window.removeEventListener("pointermove", move);
                                window.removeEventListener("pointerup", up);
                                window.removeEventListener("pointercancel", abort);
                              };
                              const move = (moveEvent: PointerEvent) => {
                                if (!dragging) {
                                  if (Math.hypot(moveEvent.clientX - originX, moveEvent.clientY - originY) < DRAG_THRESHOLD) return;
                                  dragging = true;
                                  suppressClick.current = true;
                                  setDraggingModel(model.id);
                                }
                                setDropEngine(dropTargetAt(moveEvent.clientX, moveEvent.clientY, model.id));
                              };
                              const up = (upEvent: PointerEvent) => {
                                detach();
                                setDraggingModel(null);
                                setDropEngine(null);
                                if (!dragging) return;
                                const target = dropTargetAt(upEvent.clientX, upEvent.clientY, model.id);
                                if (target) void moveModelToEngine(model.id, target);
                              };
                              const abort = () => {
                                detach();
                                setDraggingModel(null);
                                setDropEngine(null);
                              };

                              window.addEventListener("pointermove", move);
                              window.addEventListener("pointerup", up);
                              window.addEventListener("pointercancel", abort);
                            }}
                            className={`flex min-h-9 flex-wrap items-center gap-3 py-1.5 pr-2 transition-[background-color,opacity] ${draggingModel === model.id ? "select-none opacity-40" : ""} ${picked ? "bg-[color-mix(in_srgb,var(--accent)_10%,transparent)]" : ""}`}
                          >
                            {/* Two ways to start a move, one handle. Dragging is the
                                obvious one; pressing it arms the model so a group's
                                "Move here" finishes the job, which is the only
                                version of this a keyboard can perform. */}
                            <button
                              type="button"
                              onClick={() => {
                                // A drag ends with a click on whatever the pointer
                                // was over. Arming the model on the way out of a
                                // drag would leave it armed for a move the user
                                // has already made.
                                if (suppressClick.current) { suppressClick.current = false; return; }
                                setPickedModel((current) => (current?.id === model.id ? null : { id: model.id, name: model.name }));
                              }}
                              aria-pressed={picked}
                              aria-label={`Move ${model.name} to another engine`}
                              title="Drag onto another engine, or press and then choose one"
                              className={`grid size-7 shrink-0 cursor-grab place-items-center rounded-md hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)] active:cursor-grabbing ${picked ? "text-[var(--accent)]" : "text-[var(--quiet)]"}`}
                            >
                              <GripVertical size={14} />
                            </button>
                            <div className="min-w-0 flex-1">
                              <h4 className="truncate text-xs font-medium text-[var(--muted)]">{model.name}</h4>
                              {/* Facts read from the file itself. The path used to be
                                  shown here and said nothing a reader did not already
                                  have in the tooltip -- a quantisation and a context
                                  window are things they cannot get from the filename,
                                  which is usually the name of a fine-tune. */}
                              <p className="truncate text-[11px] text-[var(--quiet)]">
                                {[
                                  model.quantization,
                                  model.context_length ? `${Math.round(model.context_length / 1000)}K context` : null,
                                ].filter(Boolean).join(" · ") || "Reading model details…"}
                              </p>
                            </div>
                            {/* Only when the backend says the file is gone. It computes
                                this where it already stats the model, so the frontend
                                cannot disagree with the loader about what exists. */}
                            {!model.present && <span className="shrink-0 text-[11px] text-[var(--danger)]">File missing</span>}
                            <span className="hidden text-[11px] text-[var(--muted)] sm:block">{formatSize(model.size_bytes)}</span>
                            <button type="button" onClick={() => { setFormError(""); setPage({ view: "model", model }); }} className="grid size-8 shrink-0 place-items-center rounded-md text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]" aria-label={`Settings for ${model.name}`} title="Model settings"><Settings2 size={16} /></button>
                            <HoldToDelete onConfirm={() => void removeLocalModel(model)} label={`Remove ${model.name}`} title="Remove model from list" className="grid size-8 shrink-0 place-items-center rounded-md text-[var(--quiet)] hover:bg-[var(--raised)] hover:text-[var(--danger)]"><Trash2 size={16} /></HoldToDelete>
                          </div>;
                        })}
                      </div>
                    </div>}
                  </article>)}
                </div>}
                </> : localCategory === "stt" ? <SpeechRecognitionCategory onDownload={() => setPage({ view: "download", kind: "stt" })} onModelSettings={(model) => setPage({ view: "sttModel", model })} notify={notify} /> : <SpeechSynthesisCategory onDownload={() => setPage({ view: "download", kind: "tts" })} onSettings={() => setPage({ view: "ttsSettings" })} onVoiceSettings={(voice) => setPage({ view: "ttsVoice", voice })} notify={notify} />}
              </> : page.view === "download" ? <DownloadPage kind={page.kind} notify={notify} onModelInstalled={() => void refreshLocalModels()} />
              : page.view === "ttsSettings" ? <TtsSettingsPage notify={notify} />
              : page.view === "ttsVoice" ? <TtsVoiceSettingsPage voice={page.voice} notify={notify} />
              : page.view === "sttModel" ? <SttModelSettingsPage model={page.model} notify={notify} />
              : page.view === "downloads" ? <DownloadsPage notify={notify} />
              : page.view === "system" ? <SystemPage notify={notify} />
              : page.view === "model" ? <>
                <SettingsPageTitle>
                  <h2 className="text-[17px] font-semibold tracking-tight">{page.model.name}</h2>
                  {/* Beside the name because it decides which binary runs at all,
                      and the model cannot start without one. */}
                  <EnginePicker
                    modelId={page.model.id}
                    engineId={page.model.engine_id}
                    onChange={() => void updateLocalModelEngine()}
                    notify={notify}
                  />
                </SettingsPageTitle>
                <RuntimeSettingsPage
                  key={page.model.id}
                  modelId={page.model.id}
                  // The slider's ceiling. Read from the row already on screen
                  // rather than re-read from the file, so the limit shown by the
                  // badge above it and the limit the slider stops at cannot be
                  // two different numbers from two different reads.
                  maxContext={page.model.context_length}
                  // The model's own engine, carried in on the row, so this page
                  // names the engine the loader would use rather than guessing.
                  engineId={page.model.engine_id}
                  onEngineChanged={() => void updateLocalModelEngine()}
                  notify={notify}
                  onChange={(next) => saveLocalRuntimeSettings(page.model.id, next)}
                />
                {/* No "saved" confirmation, inline or in the corner stack. The
                    controls hold the value the user just chose, so a line
                    announcing that the app did what they asked states nothing
                    they were not already looking at. A failure is the one thing
                    here worth interrupting for, and `saveLocalRuntimeSettings`
                    reports it. */}
              </> : page.view === "appearance" ? <>
                <SettingsPageTitle><h2 className="text-[17px] font-semibold tracking-tight">Appearance</h2></SettingsPageTitle>
                <SettingsSection title="Theme">
                  <SettingRow
                    anchor="appearance.theme"
                    label="Theme"
                    description="System follows whatever your operating system is set to."
                    stacked
                    control={<ThemePicker appearance={appearance} onChange={onAppearanceChange} />}
                  />
                </SettingsSection>

                <SettingsSection title="Accent">
                  <SettingRow
                    anchor="appearance.accent"
                    label="Accent colour"
                    // Says the one thing that is not obvious from the swatches: a
                    // custom colour is not used as-is where text sits on it, so a
                    // dark choice cannot make a button's label disappear.
                    description="Used for highlights, the active view, and buttons. Anything drawn on top of the accent is picked for contrast."
                    stacked
                    control={<AccentPicker appearance={appearance} theme={resolvedTheme} onChange={onAppearanceChange} />}
                  />
                </SettingsSection>
              </> : page.view === "general" ? <>
                <SettingsPageTitle><h2 className="text-[17px] font-semibold tracking-tight">General</h2></SettingsPageTitle>
                <SettingsSection title="Instructions">
                  <SettingRow
                    anchor="general.instructions"
                    label="Instructions for ProjectZ"
                    description="Kept in mind whenever ProjectZ generates a message."
                    stacked
                    control={<textarea id="project-instructions" rows={5} value={generalSettings.instructions} onChange={(event) => updateGeneralSettings({ ...generalSettings, instructions: event.target.value })} placeholder={INSTRUCTION_EXAMPLES[instructionExampleIndex]} className="min-h-32 w-full resize-y rounded-md border border-[var(--line)] bg-[var(--rail)] px-3 py-2.5 text-[13px] leading-6 text-[var(--text)] placeholder:text-[var(--quiet)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]" />}
                  />
                </SettingsSection>

                <SettingsSection title="Response">
                  <SettingRow
                    anchor="general.notifications"
                    label="Response completions"
                    description="Get notified when ProjectZ has finished a response. Useful for long-running tasks."
                    control={<Toggle label="Notify when a response is complete" checked={generalSettings.responseNotifications} onChange={(responseNotifications) => updateGeneralSettings({ ...generalSettings, responseNotifications })} />}
                  />
                  <SettingRow
                    anchor="general.metrics"
                    label="Show token metrics"
                    description="Display prompt, completion, and generation figures under each reply."
                    control={<Toggle label="Show token metrics" checked={generalSettings.showMetrics} onChange={(showMetrics) => updateGeneralSettings({ ...generalSettings, showMetrics })} />}
                  />
                  <SettingRow
                    anchor="general.sendWith"
                    label="Send with"
                    description="Which key sends the draft. Hold the other modifier to add a line break instead."
                    control={<Segmented label="Send with" value={generalSettings.sendWith} options={["Enter", "Ctrl + Enter"]} onChange={(sendWith) => updateGeneralSettings({ ...generalSettings, sendWith })} />}
                  />
                </SettingsSection>

                <SettingsSection title="Sessions">
                  <SettingRow
                    anchor="general.clearSessions"
                    label="Clear all sessions"
                    description="Delete every conversation on this device. This cannot be undone."
                    control={<button type="button" disabled={clearingSessions} onClick={() => { setFormError(""); setConfirmClear(true); }} className="min-h-9 shrink-0 rounded-lg border border-[var(--line)] px-3 text-[13px] text-[var(--muted)] transition-colors hover:border-[var(--danger)] hover:bg-[var(--raised)] hover:text-[var(--danger)] disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]">Clear…</button>}
                  />
                </SettingsSection>
              </> : page.view === "preferences" ? <>
                <SettingsPageTitle><h2 className="text-[17px] font-semibold tracking-tight">Preferences</h2></SettingsPageTitle>
                {/* Both model choices are the same kind of setting -- a model
                    for a background task -- so they are drawn by the same flat
                    row rules as every `SettingsSection`; there is no panel here
                    to clip the pickers' popovers either. */}
                <div className="divide-y divide-[var(--line)]">
                  <div className="[&>*:first-child]:!border-0" {...{ [ANCHOR_ATTRIBUTE]: "preferences.titleModels", [MARKABLE_ATTRIBUTE]: "" }}>
                    <TitleModelPicker
                      models={preferences.sessionTitleModels}
                      onChange={(sessionTitleModels) => { setFormError(""); onPreferencesChange({ ...preferences, sessionTitleModels }); }}
                      groups={providerModelGroups}
                      localModels={localModels}
                    />
                  </div>
                  <div className="flex min-h-[58px] items-center gap-3 px-4 py-3.5" {...{ [ANCHOR_ATTRIBUTE]: "preferences.subagentModel", [MARKABLE_ATTRIBUTE]: "" }}>
                    <div className="min-w-0 flex-1">
                      <h3 className="text-[13px]">Sub-agent model</h3>
                      <p className="mt-0.5 text-[12px] leading-5 text-[var(--muted)]">The model spawned agents run on. Defaults to whatever the chat is using.</p>
                    </div>
                    <SingleModelPicker
                      value={preferences.subagentModel ?? null}
                      onChange={(subagentModel) => { setFormError(""); onPreferencesChange({ ...preferences, subagentModel }); }}
                      groups={providerModelGroups}
                      localModels={localModels}
                      emptyLabel="Same as chat"
                      label="Default sub-agent model"
                    />
                  </div>
                  <div className="flex min-h-[58px] items-center gap-3 px-4 py-3.5" {...{ [ANCHOR_ATTRIBUTE]: "preferences.visionModel", [MARKABLE_ATTRIBUTE]: "" }}>
                    <div className="min-w-0 flex-1">
                      <h3 className="text-[13px]">Vision model</h3>
                      <p className="mt-0.5 text-[12px] leading-5 text-[var(--muted)]">The model an image is shown to when a file is read. Leave off and images cannot be read.</p>
                    </div>
                    <SingleModelPicker
                      value={preferences.visionModel ?? null}
                      onChange={(visionModel) => { setFormError(""); onPreferencesChange({ ...preferences, visionModel }); }}
                      groups={providerModelGroups}
                      localModels={localModels}
                      emptyLabel="Off"
                      label="Default vision model"
                    />
                  </div>
                </div>

                <SettingsSection title="Context">
                  <SettingRow
                    anchor="preferences.compaction"
                    label="Context compaction"
                    description="How ProjectZ trims a long conversation before sending it. Off keeps everything; Normal elides old tool output; Fast also drops older turns."
                    control={
                      <Segmented
                        label="Context compaction"
                        value={COMPACTION_LABELS[preferences.compaction]}
                        options={["Off", "Normal", "Fast"]}
                        onChange={(label) => {
                          // The stored value is the lowercase wire name the backend
                          // parses, so the display label is mapped back rather than
                          // saved directly.
                          const next = (Object.keys(COMPACTION_LABELS) as Compaction[]).find((key) => COMPACTION_LABELS[key] === label) ?? "normal";
                          setFormError("");
                          onPreferencesChange({ ...preferences, compaction: next });
                        }}
                      />
                    }
                  />
                </SettingsSection>
              </> : page.view === "skills" ? <SkillsPage notify={notify} onOpenForm={(skill) => setPage({ view: "skill", slug: skill?.slug ?? null })} /> : page.view === "mcp" ? <McpSettingsPage notify={notify} /> : page.view === "skill" ? <SkillDetailPage key={editingSkill ?? "new"} slug={editingSkill} onSaved={(slug) => { if (slug !== editingSkill) setPage({ view: "skill", slug }); }} onCancel={() => setPage({ view: "skills" })} onDeleted={() => setPage({ view: "skills" })} notify={notify} /> : <>
                <SettingsPageTitle><h2 className="text-[17px] font-semibold tracking-tight">{page.endpoint ? "Edit provider" : "Add provider"}<FlashHighlight anchor="providers.add" /></h2></SettingsPageTitle>
                <form onSubmit={(event) => void saveEndpoint(event)} className="max-w-xl space-y-4">
                  <label className="block text-sm text-[var(--muted)]">Provider name<input ref={nameRef} required value={draft.name} onChange={(event) => setDraft({ ...draft, name: event.target.value })} className="mt-1.5 h-10 w-full rounded-md border border-[var(--line)] bg-[var(--rail)] px-3 text-[13px] text-[var(--text)] outline-none placeholder:text-[var(--quiet)] focus:border-[var(--accent)]" placeholder="e.g. Local Ollama" /></label>
                  <label className="block text-sm text-[var(--muted)]">Base URL<input type="url" required value={draft.base_url} onChange={(event) => setDraft({ ...draft, base_url: event.target.value })} className="mt-1.5 h-10 w-full rounded-md border border-[var(--line)] bg-[var(--rail)] px-3 text-[13px] text-[var(--text)] outline-none placeholder:text-[var(--quiet)] focus:border-[var(--accent)]" placeholder="https://api.example.com/v1" /></label>
                  <label className="block text-sm text-[var(--muted)]">API key <span className="text-[var(--quiet)]">({page.endpoint?.has_api_key ? "saved, leave blank to keep" : "optional"})</span><input type={showKey ? "text" : "password"} autoComplete="new-password" value={draft.api_key} onChange={(event) => setDraft({ ...draft, api_key: event.target.value })} className="mt-1.5 h-10 w-full rounded-md border border-[var(--line)] bg-[var(--rail)] px-3 text-[13px] text-[var(--text)] outline-none placeholder:text-[var(--quiet)] focus:border-[var(--accent)]" placeholder="Paste provider key" /><button type="button" onClick={() => setShowKey((value) => !value)} className="mt-1 min-h-8 text-xs text-[var(--muted)] hover:text-[var(--text)]">{showKey ? "Hide key" : "Show key"}</button></label>
                  <p className="text-xs text-[var(--quiet)]">Model discovery runs when you test the connection.</p>
                  {/* The one message that stays inline. It is this form's own
                      validation rather than an outcome, and it points at the field
                      by focusing it -- which a row in the corner stack cannot do.
                      Everything else, including a failed save, is a notification. */}
                  {formError && <p role="alert" className="text-sm text-[var(--danger)]">{formError}</p>}
                  <div className="flex flex-wrap gap-2 border-t border-[var(--line)] pt-5">
                    <button type="submit" disabled={loading} className="inline-flex min-h-9 items-center gap-1.5 rounded-lg bg-[var(--accent)] px-3.5 text-[13px] font-semibold text-[var(--accent-ink)] transition-opacity hover:opacity-90 disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]">{loading ? "Saving…" : page.endpoint ? "Save changes" : "Add provider"}</button>
                    <button type="button" onClick={() => void testEndpoint(draft)} disabled={testingId === draft.id || !draft.base_url.trim()} className="inline-flex min-h-9 items-center gap-1.5 rounded-lg border border-[var(--line)] px-3 text-[13px] text-[var(--text)] transition-colors hover:bg-[var(--raised)] disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"><Zap size={15} />{testingId === draft.id ? "Testing…" : "Test and find models"}</button>
                    <button type="button" onClick={() => setPage({ view: "providers" })} className="min-h-9 rounded-lg px-3 text-[13px] text-[var(--muted)] transition-colors hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]">Cancel</button>
                  </div>
                </form>
              </>}
              </SettingsSearchScope>
            </div>
          </div>
        </div>
      </div>
      {confirmClear && <div className="fixed inset-0 z-[150] grid place-items-center bg-black/65 p-4">
        <section role="dialog" aria-modal="true" aria-labelledby="clear-sessions-title" className="w-full max-w-sm rounded-lg border border-[var(--line)] bg-[var(--panel)] p-5 shadow-2xl" onKeyDown={(event) => { if (event.key === "Escape" && !clearingSessions) setConfirmClear(false); }}>
          <h2 id="clear-sessions-title" className="text-[17px] font-semibold tracking-tight">Delete every session?</h2>
          <p className="mt-2 text-[13px] leading-5 text-[var(--muted)]">This removes all conversations stored on this device and cannot be undone. Providers, local models, and settings are kept.</p>
          <div className="mt-5 flex justify-end gap-2">
            <button type="button" onClick={() => setConfirmClear(false)} disabled={clearingSessions} className="min-h-9 rounded-lg border border-[var(--line)] px-3.5 text-[13px] text-[var(--muted)] transition-colors hover:bg-[var(--raised)] disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]">Cancel</button>
            <button type="button" onClick={() => void clearSessions()} disabled={clearingSessions} className="inline-flex min-h-9 items-center gap-1.5 rounded-lg bg-[var(--danger)] px-3.5 text-[13px] font-semibold text-[var(--page)] transition-opacity hover:opacity-90 disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--danger)]">{clearingSessions && <LoaderCircle size={15} className="animate-spin" />}{clearingSessions ? "Deleting…" : "Delete all"}</button>
          </div>
        </section>
      </div>}
    </dialog>
  );
}
