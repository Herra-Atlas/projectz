import { useEffect, useState } from "react";
import { ChevronRight, ExternalLink } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import HelpMark from "../HelpMark";
import { ANCHOR_ATTRIBUTE, FlashHighlight, MARKABLE_ATTRIBUTE, SettingRow, SettingsSection, Segmented, SettingsPageTitle, Toggle } from "../SettingsSection";
import SingleModelPicker from "../ModelSelectionPicker";
import LevelPicker from "./LevelPicker";
import type { AssistLevelBounds, AssistSettings } from "../../features/assist/types";
import type { ModelGroup } from "../../features/models/useModelRegistry";
import type { LocalModel } from "../../features/models/types";
import type { Notify } from "../../features/notifications/types";
import type { AssistStore } from "../../features/assist/useAssist";

/**
 * The confidence floors the picker offers, in the order they are drawn.
 *
 * Only three, because this is a judgement rather than a dial: a wrong card unasked costs more
 * than ten useful ones earn, so the useful range is narrow. A number between them is still
 * shown as the nearest label; editing it moves the record to `custom` like every other field.
 */
const CONFIDENCE = [
  { label: "Eager", value: 0.4 },
  { label: "Balanced", value: 0.55 },
  { label: "Strict", value: 0.7 },
] as const;

const CONFIDENCE_LABELS = CONFIDENCE.map((option) => option.label);

/** The confidence floor, as the label nearest to it. */
function confidenceLabel(value: number) {
  return CONFIDENCE.reduce((best, option) =>
    Math.abs(option.value - value) < Math.abs(best.value - value) ? option : best
  ).label;
}

/** What the level in force is willing to help with, in the backend's own words. */
function levelKinds(bounds: AssistLevelBounds) {
  const kinds = bounds.kinds.join(", ");
  return bounds.locate
    ? `${kinds}. Picks what is worth it in two passes over the main content.`
    : kinds;
}

type AssistSettingsPageProps = {
  notify: Notify;
  /**
   * The record and its live status, read by the settings modal when the dialog opens.
   *
   * Read there rather than here so the read has happened before the page is even clicked:
   * a read that starts on mount cannot start before the click, and the click is exactly when
   * the user is waiting.
   */
  store: AssistStore;
  /** The provider groups the settings modal already loaded, for the model picker. */
  groups: ModelGroup[];
  localModels: LocalModel[];
};

/**
 * Settings for helping without being asked.
 *
 * The page follows the order the feature is set up in: switch it on, choose what answers,
 * connect the browser, then tune when it is allowed to speak.
 *
 * **Explanations live behind the `?`.** Every row here had a paragraph under it, which made a
 * page nobody could scan; what is left is one short line per row and the reasoning behind the
 * `?` for whoever asks for it. There is no try-it box and no source list: an answer is drawn by
 * the browser, on the page it belongs to, and a source that is not built is not a row.
 */
export default function AssistSettingsPage({ notify, store, groups, localModels }: AssistSettingsPageProps) {
  const { status, error, save, setEnabled } = store;
  // The in-progress edit, held here rather than in the hook: a control that wrote to the backend
  // on every keystroke would round trip per character, and a number field is committed on blur
  // for exactly that reason. Cleared after every save, so what is on screen is the stored record
  // -- including a value the backend clamped.
  const [draft, setDraft] = useState<AssistSettings | null>(null);
  // The tuning numbers live behind a disclosure: the page's job on a normal visit is the
  // switch, the model, the browser and how hard it works, and a wall of thresholds buries them.
  const [advanced, setAdvanced] = useState(false);

  const settings = draft ?? status?.settings ?? null;

  useEffect(() => {
    if (error) notify("error", `Assist settings could not be read: ${error}`);
    // One report per error, not per render: `notify` is stable and the error only changes when
    // a read fails.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [error]);

  // `settings` comes from the draft or the record; `status` is what carries the live facts
  // (bridge, level bounds, hour's spending). The form needs both, so the page waits for the
  // pair -- but the pair arrives early, because the modal started the read when it opened
  // and `assist_settings` seeds the record before `assist_status` has finished.
  if (!settings || !status) {
    return (
      <SettingsPageTitle>
        <h2 className="text-[17px] font-semibold tracking-tight">Assist</h2>
      </SettingsPageTitle>
    );
  }

  const commit = (next: AssistSettings) => {
    setDraft(next);
    void save(next).then((updated) => {
      setDraft(null);
      if (!updated) notify("error", "Assist settings could not be saved.");
    });
  };

  /**
   * Writes a threshold, and says the record is the user's own from here on.
   *
   * A preset owns these fields, so a record that kept `normal` beside a number somebody had
   * changed would be a picker showing a level that is not the one running -- and the next click
   * on `Normal` would appear to change nothing. Editing a number *is* asking for a custom one.
   */
  const commitThreshold = (next: Partial<AssistSettings>) => {
    commit({ ...settings, ...next, level: "custom" });
  };

  const toggleSubject = (subject: string) => {
    const subjects = settings.subjects.includes(subject)
      ? settings.subjects.filter((entry) => entry !== subject)
      : [...settings.subjects, subject];
    commit({ ...settings, subjects });
  };

  const openSetup = () => {
    const url = `http://127.0.0.1:${status.bridge.port ?? 43128}/`;
    void openUrl(url).catch((reason: unknown) => notify("error", `Could not open ${url}: ${String(reason)}`));
  };

  const connection = status.bridge.error
    ? `The socket could not start: ${status.bridge.error}`
    : !status.bridge.running
      ? "Starting…"
      : status.bridge.clients.length === 0
        ? "No browser is paired yet"
        : status.bridge.clients.length === 1
          ? `Paired with one browser${status.bridge.clients[0] ? ` · ${status.bridge.clients[0]}` : ""}`
          : `Paired with ${status.bridge.clients.length} browsers`;

  return (
    <>
      {/* The switch sits where Skills puts "New skill": top right of the page header,
          rather than as a row of its own. It is the one control that decides whether
          this page's subject exists at all, so it should not be scrolled away with the
          thresholds. */}
      <div className="mb-6 flex flex-wrap items-start justify-between gap-3 border-b border-[var(--line)] pb-5">
        <div className="min-w-0">
          <h2 className="text-[17px] font-semibold tracking-tight">Assist</h2>
          {/* Only the on state is written down. A switch that is off already says
              so itself, and a line that repeated it was a line nobody read. */}
          {settings.enabled && (
            <p className="mt-1 flex items-center gap-1.5 text-[12px] text-[var(--muted)]">
              <span>On. A page that asks something gets a card.</span>
              <HelpMark hint="The switch that matters. Off, no page is read at all — the browser extension is told to stop, and nothing reaches a model. The same switch is in the tray's menu and in the extension's own popup, and all three move this one value." />
            </p>
          )}
        </div>
        <div className="shrink-0 pt-0.5" {...{ [ANCHOR_ATTRIBUTE]: "assist.enabled", [MARKABLE_ATTRIBUTE]: "" }}>
          <FlashHighlight anchor="assist.enabled" />
          <Toggle
            label="Help without being asked"
            checked={settings.enabled}
            onChange={(next) => void setEnabled(next)}
          />
        </div>
      </div>

      <SettingsSection title="Model">
        <SettingRow
          anchor="assist.model"
          label="The model that answers"
          description={status.model ?? "Nothing chosen yet"}
          hint="Assist uses a model that is already available and never loads one itself: the app runs a single local model server, so loading assist's model would unload whichever model your chat is using. A local model therefore has to be the one already loaded."
          control={
            <SingleModelPicker
              value={settings.model}
              onChange={(model) => commit({ ...settings, model })}
              groups={groups}
              localModels={localModels}
              emptyLabel="Nothing yet"
              label="Assist model"
            />
          }
        />
        {/* The readiness line (`status.modelReady`) is deliberately not drawn: it
            describes a switch that is off as "not loaded", which is not a problem to
            report -- nothing was set up yet. The choice itself is on the row above. */}
      </SettingsSection>

      <SettingsSection title="Your browser">
        <SettingRow
          anchor="assist.setup"
          label="Set up the extension"
          description={connection}
          hint="The app serves its own page on this port. That page hands over the extension as a download, and once the extension is loaded the page pairs it automatically — the token is carried in the page, so there is nothing to copy. The same port is the socket the extension talks to, so nothing else needs opening."
          control={
            <button
              type="button"
              onClick={openSetup}
              className="inline-flex min-h-9 items-center gap-1.5 rounded-lg border border-[var(--line)] px-3 text-[13px] text-[var(--text)] transition-colors hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
            >
              <ExternalLink size={14} />
              Open the setup page
            </button>
          }
        />
      </SettingsSection>

      <SettingsSection title="When to answer">
        {/* The picker is not a `SettingRow` -- three cards and a spend line are not a
            label-and-control pair -- so its anchor rides on the wrapper that search
            points at. */}
        <div {...{ [ANCHOR_ATTRIBUTE]: "assist.performance", [MARKABLE_ATTRIBUTE]: "" }}>
          <FlashHighlight anchor="assist.performance" />
          <LevelPicker
            level={settings.level}
            bounds={status.levelBounds}
            onChoose={(level) => commit({ ...settings, level })}
          />
        </div>
        <SettingRow
          stacked
          anchor="assist.subjects"
          label="Worth more of your attention"
          description="Ticked subjects are weighted, in the order you tick them."
          hint="A weight, not a gate: a question outside the list can still be worth answering, and the list is what the model is told to lean towards. The number beside a ticked subject is the order it is given in."
          control={
            <div className="flex flex-wrap gap-1.5">
              {status.suggestedSubjects.map((subject) => {
                const chosen = settings.subjects.includes(subject);
                const position = settings.subjects.indexOf(subject) + 1;
                return (
                  <button
                    key={subject}
                    type="button"
                    aria-pressed={chosen}
                    onClick={() => toggleSubject(subject)}
                    className={`min-h-8 rounded-full border px-3 text-[12px] transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)] ${
                      chosen
                        ? "border-[color-mix(in_srgb,var(--accent)_45%,var(--line))] bg-[color-mix(in_srgb,var(--accent)_12%,transparent)] text-[var(--accent)]"
                        : "border-[var(--line)] text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]"
                    }`}
                  >
                    {subject}
                    {chosen && <span className="ml-1.5 text-[10px] opacity-70">{position}</span>}
                  </button>
                );
              })}
            </div>
          }
        />
      </SettingsSection>

      <button
        type="button"
        onClick={() => setAdvanced((open) => !open)}
        aria-expanded={advanced}
        className="mt-7 flex w-full items-center gap-1.5 text-left text-[12px] font-semibold tracking-tight text-[var(--muted)] transition-colors hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
      >
        <ChevronRight
          size={14}
          className={`shrink-0 transition-transform ${advanced ? "rotate-90" : ""}`}
        />
        Advanced
      </button>

      {advanced && (
        <>
          <SettingsSection title="Behaviour">
            <SettingRow
              stacked
              anchor="assist.hint"
              label="Anything to keep in mind"
              description="Sent with every request as a preference."
              hint="A hint, never a filter. It steers what is worth interrupting for and can never block an answer: a rule that could refuse would be a feature that silently stops working, and you could not tell that from a page with nothing on it."
              control={
                <textarea
                  value={settings.hint}
                  onChange={(event) => setDraft({ ...settings, hint: event.target.value })}
                  onBlur={() => commit(settings)}
                  rows={2}
                  className="min-h-[58px] w-full resize-y rounded-md border border-[var(--line)] bg-[var(--rail)] px-3 py-2.5 text-[13px] leading-6 text-[var(--text)] placeholder:text-[var(--quiet)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
                  placeholder="e.g. maths and science are the ones I get stuck on"
                />
              }
            />
            <SettingRow
              anchor="assist.kinds"
              label="What it may help with"
              description={levelKinds(status.levelBounds)}
              hint="A level is a breadth as well as a spend. Efficient answers questions only. Normal adds a short explanation, a check and a translation. High also offers a summary of a long section, and looks at the page in two passes — a quick one that points at what is worth helping with, then a focused one that writes it. That two-pass shape is what makes a small model useful on a long page: pointing at short text is a task it can do, reading everything at once is not."
            />
            <SettingRow
              anchor="assist.confidence"
              label="Only speak when sure"
              description={`${Math.round(settings.minConfidence * 100)}% confidence or more`}
              hint="The floor is enforced by the app, not merely asked for in the prompt. A model is already told to stay quiet when unsure, but a 2B's judgement about its own uncertainty is the least reliable thing it does — and a wrong card nobody asked for costs more trust than ten useful ones earn. Raising this makes assist quieter; lowering it makes it more willing. Editing it moves the level to Custom."
              control={
                <Segmented
                  value={confidenceLabel(settings.minConfidence)}
                  options={CONFIDENCE_LABELS}
                  onChange={(label) => {
                    const chosen = CONFIDENCE.find((option) => option.label === label);
                    if (chosen) commitThreshold({ minConfidence: chosen.value });
                  }}
                  label="Only speak when sure"
                />
              }
            />
            <SettingRow
              anchor="assist.thinking"
              label="Let the model think before answering"
              description={settings.thinking ? "On. It may reason first." : "Off. It answers directly."}
              hint="Measured, not guessed: on a local 2B, thinking spent the whole token ceiling and returned nothing — twice — while the same page answered correctly in a fraction of the tokens with it off. Assist's task is a short answer in a strict shape, and a small model's thinking is mostly it re-reading the instructions. A large model is a different story, so this is a switch rather than a rule. It also does not affect the retry: a reply that cannot be read is asked again either way, and one that was cut off is asked again without thinking."
              control={
                <Toggle
                  label="Let the model think before answering"
                  checked={settings.thinking}
                  onChange={(thinking) => commit({ ...settings, thinking })}
                />
              }
            />
          </SettingsSection>

          <SettingsSection title="Limits">
            <SettingRow
              anchor="assist.minChars"
              label="Shortest to look at"
              description="A block below this many characters is ignored."
              hint="The shortest thing worth answering is short: a sum like 14 - 8 = is seven characters. This is the floor the whole page is filtered by, so raising it is the quickest way to make assist quieter. The same number is handed to the browser extension, so it stops short blocks crossing the wire at all rather than sending them to be refused — one floor, not two. No preset changes it."
              control={
                <NumberField
                  value={settings.minChars}
                  min={4}
                  max={400}
                  step={4}
                  label="Shortest block in characters"
                  onCommit={(minChars) => commitThreshold({ minChars })}
                />
              }
            />
            <SettingRow
              anchor="assist.candidates"
              label="Candidates considered"
              description="The short list's ceiling for one look."
              hint="The gate ranks what it found and keeps this many. A page with more questions than this keeps the strongest, and among equal evidence the ones nearest your screen. A preset owns it; editing it moves the level to Custom."
              control={
                <NumberField
                  value={settings.maxCandidates}
                  min={1}
                  max={32}
                  step={1}
                  label="Candidates considered"
                  onCommit={(maxCandidates) => commitThreshold({ maxCandidates })}
                />
              }
            />
            <SettingRow
              anchor="assist.perRequest"
              label="Candidates per request"
              description="How many blocks one request carries."
              hint="This is the batch size. Ten candidates at five per request is two requests, if the request budget allows two. It may be larger than answers per reply: the model can be shown more blocks than it is allowed to answer. It cannot be larger than candidates considered."
              control={
                <NumberField
                  value={settings.candidatesPerRequest}
                  min={1}
                  max={32}
                  step={1}
                  label="Candidates per request"
                  onCommit={(candidatesPerRequest) => commitThreshold({ candidatesPerRequest })}
                />
              }
            />
            <SettingRow
              anchor="assist.answers"
              label="Answers per reply"
              description="The most cards one reply may carry."
              hint="The parser enforces it, so a model that ignores the instruction cannot cover the page. It is clamped to candidates per request, because a reply cannot answer more blocks than its own request carried."
              control={
                <NumberField
                  value={settings.maxAnswers}
                  min={1}
                  max={16}
                  step={1}
                  label="Answers per reply"
                  onCommit={(maxAnswers) => commitThreshold({ maxAnswers })}
                />
              }
            />
            <SettingRow
              anchor="assist.tokens"
              label="Token ceiling"
              description="The reply's size limit, shared with the model's thinking."
              hint="A model with a reasoning channel spends this budget on its thinking before it writes anything, so a ceiling that only fits the answer stops it mid-sentence. A reply that fills it without finishing is asked again once with thinking switched off."
              control={
                <NumberField
                  value={settings.maxTokens}
                  min={128}
                  max={8192}
                  step={128}
                  label="Token ceiling"
                  onCommit={(maxTokens) => commitThreshold({ maxTokens })}
                />
              }
            />
            <SettingRow
              anchor="assist.requests"
              label="Requests per look"
              description="How many batches one look may send."
              hint="Candidates are split into batches of the size above, and this is how many batches may be spent. Ten candidates at five per request needs two. A page whose first batch answers nothing stops there even if more are allowed."
              control={
                <NumberField
                  value={settings.maxRequests}
                  min={1}
                  max={18}
                  step={1}
                  label="Requests per look"
                  onCommit={(maxRequests) => commitThreshold({ maxRequests })}
                />
              }
            />
            <SettingRow
              anchor="assist.perHour"
              label="Answers an hour"
              description="A ceiling on what this costs."
              hint="Counted in requests, not answers: one page is one request however many questions are on it, and a request that fails costs the same as one that succeeds. Anything over the limit is skipped until the hour passes. The ceiling is this app's on purpose, and the browser only ever reads it — a limit the browser could set is a limit a modified browser could lift, and this is the side that pays."
              control={
                <NumberField
                  value={settings.maxPerHour}
                  min={1}
                  max={240}
                  step={1}
                  label="Answers per hour"
                  onCommit={(maxPerHour) => commitThreshold({ maxPerHour })}
                />
              }
            />
          </SettingsSection>

          <SettingsSection title="Diagnostics">
            <SettingRow
              anchor="assist.dryRun"
              label="Show me what it would have answered"
              description="Everything runs; nothing is drawn."
              hint="Dry run. The whole pipeline runs including the model, the verdict comes back, and no card is drawn on any page. The extension logs what it would have shown. This is how the thresholds above get set from real pages instead of from guesses."
              control={<Toggle label="Dry run" checked={settings.dryRun} onChange={(dryRun) => commit({ ...settings, dryRun })} />}
            />
            <SettingRow
              anchor="assist.usage"
              label="This session"
              description={`${status.callsPastHour === 1 ? "1 request" : `${status.callsPastHour} requests`} in the last hour, against the limit above.`}
              hint="Reset when the app restarts. What assist spent over a longer period is on the Statistics page, beside everything else."
            />
          </SettingsSection>
        </>
      )}
    </>
  );
}

/**
 * A number that is written when the user leaves it.
 *
 * Committed on blur rather than on every keystroke, because a field that saved as it was typed
 * would store `1` on the way to `14` and spend the moment in between with a threshold nobody
 * chose. Empty is ignored rather than read as zero: a cleared field is someone about to type a
 * new value, not a request for the floor.
 */
function NumberField({
  value,
  min,
  max,
  step,
  label,
  onCommit,
}: {
  value: number;
  min: number;
  max: number;
  step: number;
  label: string;
  onCommit: (next: number) => void;
}) {
  const [text, setText] = useState(String(value));
  // Follows the stored value when it changes from elsewhere -- a clamp on the backend, or the
  // switch being moved in another window.
  useEffect(() => setText(String(value)), [value]);
  return (
    <input
      type="number"
      inputMode="numeric"
      aria-label={label}
      value={text}
      min={min}
      max={max}
      step={step}
      onChange={(event) => setText(event.target.value)}
      onBlur={() => {
        const parsed = Number.parseInt(text, 10);
        if (!Number.isFinite(parsed)) {
          setText(String(value));
          return;
        }
        if (parsed !== value) onCommit(parsed);
      }}
      className="h-9 w-[92px] rounded-md border border-[var(--line)] bg-[var(--rail)] px-2.5 text-[13px] text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
    />
  );
}
