import { useEffect, useState } from "react";
import { ChevronRight, LoaderCircle, Play } from "lucide-react";
import HelpMark from "../HelpMark";
import { ANCHOR_ATTRIBUTE, FlashHighlight, MARKABLE_ATTRIBUTE, SettingRow, SettingsPageTitle, SettingsSection, Toggle } from "../SettingsSection";
import TitleModelPicker from "../TitleModelPicker";
import { selectionId, useModelOptions } from "../ModelSelectionPicker";
import TasteDocument from "./TasteDocument";
import TasteRunReport from "./TasteRunReport";
import { dateTime } from "../../features/taste/tasteFormat";
import type { TasteStore } from "../../features/taste/useTaste";
import type { ModelGroup } from "../../features/models/useModelRegistry";
import type { LocalModel } from "../../features/models/types";
import type { Notify } from "../../features/notifications/types";

type TasteSettingsPageProps = {
  notify: Notify;
  /**
   * The record, the profile and the history, read by the settings modal when the dialog
   * opens rather than when this section is clicked -- the click is when the user is waiting.
   */
  store: TasteStore;
  /** The provider groups the settings modal already loaded, for the model picker. */
  groups: ModelGroup[];
  localModels: LocalModel[];
  /**
   * Opens the right panel's Taste transcript.
   *
   * Called only from the Run now button; a scheduled run never opens a panel on
   * its own. The panel is where a run is watched, so a manual run should show it.
   */
  onTasteRun?: () => void;
};

/**
 * Settings for the learned profile.
 *
 * The page follows the settings shape: the switch is the one control in the header,
 * the main sections are the model and the schedule, the profile is the content,
 * and the run history lives behind a disclosure rather than occupying the page.
 * A fact is editable and deletable in place -- an inference about someone should
 * never be something they cannot correct.
 */
export default function TasteSettingsPage({ notify, store, groups, localModels, onTasteRun }: TasteSettingsPageProps) {
  const { status, history, error, busy, save, runNow, updateItem, deleteItem } = store;
  const [days, setDays] = useState("4");
  const [runsOpen, setRunsOpen] = useState(false);
  // The same option list the picker draws. A model that was removed, deleted or
  // disabled is absent from it, so a stored selection can be checked against it
  // without a second source of truth.
  const options = useModelOptions(groups, localModels);

  // The backend clamps the interval and moves the next run when the cadence
  // changes, so the field follows the stored value after every read rather than
  // holding an edit that may not be what was saved.
  const intervalDays = status?.settings.intervalDays;
  useEffect(() => {
    if (intervalDays !== undefined) setDays(String(intervalDays));
  }, [intervalDays]);

  if (!status) {
    return (
      <SettingsPageTitle>
        <h2 className="text-[17px] font-semibold tracking-tight">Taste</h2>
      </SettingsPageTitle>
    );
  }

  const settings = status.settings;
  const running = status.running;
  const available = new Set(options.map((option) => selectionId(option.selection)));
  const hasUsableModel = settings.models.some((selection) => available.has(selectionId(selection)));

  const saveInterval = () => {
    const parsed = Number.parseInt(days, 10);
    const value = Number.isFinite(parsed) ? Math.min(30, Math.max(1, parsed)) : settings.intervalDays;
    setDays(String(value));
    if (value !== settings.intervalDays) {
      void save({ ...settings, intervalDays: value });
    }
  };

  const run = async () => {
    // Only a manual run opens the panel. A scheduled run is background work and
    // must never pull a panel over what the user is reading.
    onTasteRun?.();
    const result = await runNow();
    if (!result) {
      notify("error", "Taste could not run.");
      return;
    }
    if (result.status === "failed") {
      notify("error", `Taste failed: ${result.reason ?? "The run did not finish."}`);
    } else if (result.status === "skipped") {
      notify("success", "Nothing new to learn");
    } else if (result.added + result.updated === 0) {
      notify("success", "Nothing new to learn");
    } else {
      notify("success", `Taste updated · ${result.added} added, ${result.updated} reinforced`);
    }
  };

  const update = async (id: number, text: string) => {
    try {
      await updateItem(id, text);
    } catch (reason) {
      notify("error", `Could not save that fact: ${String(reason)}`);
      throw reason;
    }
  };

  const remove = async (id: number) => {
    try {
      await deleteItem(id);
    } catch (reason) {
      notify("error", `Could not delete that fact: ${String(reason)}`);
      throw reason;
    }
  };

  const nextRunLabel = !settings.enabled
    ? "Nothing runs while Taste is off"
    : running
      ? "Running now"
      : settings.nextRunAt
        ? dateTime(settings.nextRunAt)
        : "Not scheduled";

  const earlierRuns = history
    .filter((entry) => entry.id !== status.lastRun?.id)
    .slice(0, 10);

  return (
    <>
      {/* One line, like every other settings page: the title, the question mark and
          the switch. The old subtitle was a second row for a fact the switch already
          states, and it made this page's header taller than its neighbours'. */}
      <SettingsPageTitle>
        <div className="flex min-w-0 items-center gap-1.5">
          <h2 className="text-[17px] font-semibold tracking-tight">Taste</h2>
          <HelpMark hint="A run reads only text you wrote in conversations it has not read yet — never assistant replies, reasoning, tool output or sub-agent transcripts. Each stored fact carries the model's confidence. Off, nothing new is read and no learned context is sent with chats. On, learned facts ride with every chat. Taste needs an enabled model to turn on." />
        </div>
        <div
          className="shrink-0"
          title={!settings.enabled && !hasUsableModel ? "Choose an enabled taste model first." : undefined}
          {...{ [ANCHOR_ATTRIBUTE]: "taste.enabled", [MARKABLE_ATTRIBUTE]: "" }}
        >
          <FlashHighlight anchor="taste.enabled" />
          <Toggle
            label="Learn about you"
            checked={settings.enabled}
            disabled={!settings.enabled && !hasUsableModel}
            onChange={(enabled) => {
              if (enabled && !hasUsableModel) {
                notify("error", "Choose an enabled taste model first.");
                return;
              }
              void save({ ...settings, enabled });
            }}
          />
        </div>
      </SettingsPageTitle>

      <SettingsSection title="Model">
        <TitleModelPicker
          anchor="taste.models"
          label="Taste model"
          description={
            hasUsableModel
              ? "Extracts short facts from your own messages."
              : settings.models.length > 0
                ? "The selected model is no longer available. Choose an enabled model to turn Taste on."
                : "Choose an enabled model to turn Taste on."
          }
          models={settings.models}
          onChange={(models) => void save({ ...settings, models })}
          groups={groups}
          localModels={localModels}
          max={3}
        />
      </SettingsSection>

      <SettingsSection title="Schedule">
        <SettingRow
          anchor="taste.interval"
          label="Every"
          description="Days between runs."
          control={
            <div className="flex shrink-0 items-center gap-2">
              <input
                aria-label="Days between taste runs"
                type="number"
                min={1}
                max={30}
                value={days}
                onChange={(event) => setDays(event.target.value)}
                onBlur={saveInterval}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    event.preventDefault();
                    event.currentTarget.blur();
                  }
                }}
                className="h-9 w-16 rounded-md border border-[var(--line)] bg-[var(--rail)] px-2 text-center text-[13px] tabular-nums text-[var(--text)] outline-none focus:border-[var(--accent)]"
              />
              <span className="text-[12px] text-[var(--muted)]">days</span>
            </div>
          }
        />
        <SettingRow
          anchor="taste.runNow"
          label="Next run"
          description={nextRunLabel}
          control={
            <button
              type="button"
              onClick={() => void run()}
              disabled={busy || running || !hasUsableModel}
              title={!hasUsableModel ? "Choose an enabled taste model first." : undefined}
              className="inline-flex min-h-9 shrink-0 items-center gap-1.5 rounded-lg border border-[var(--line)] px-3 text-[13px] text-[var(--text)] transition-colors hover:bg-[var(--raised)] disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
            >
              {running ? <LoaderCircle size={14} className="animate-spin" /> : <Play size={14} />}
              {running ? "Learning…" : "Run now"}
            </button>
          }
        />
      </SettingsSection>

      {error && (
        <p role="alert" className="mt-3 px-4 text-[12px] text-[var(--danger)]">
          {error}
        </p>
      )}

      <SettingsSection title="What ProjectZ knows">
        <div {...{ [ANCHOR_ATTRIBUTE]: "taste.profile", [MARKABLE_ATTRIBUTE]: "" }}>
          <FlashHighlight anchor="taste.profile" />
          <TasteDocument items={status.items} onUpdate={update} onDelete={remove} />
        </div>
      </SettingsSection>

      <button
        type="button"
        {...{ [ANCHOR_ATTRIBUTE]: "taste.runs", [MARKABLE_ATTRIBUTE]: "" }}
        onClick={() => setRunsOpen((open) => !open)}
        aria-expanded={runsOpen}
        className="mt-7 flex w-full items-center gap-1.5 text-left text-[12px] font-semibold tracking-tight text-[var(--muted)] transition-colors hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
      >
        <FlashHighlight anchor="taste.runs" />
        <ChevronRight
          size={14}
          className={`shrink-0 transition-transform ${runsOpen ? "rotate-90" : ""}`}
        />
        Runs
      </button>

      {runsOpen && (
        <>
          <SettingsSection title="Latest run">
            {status.lastRun ? (
              <TasteRunReport run={status.lastRun} />
            ) : (
              <p className="px-4 py-5 text-[12px] text-[var(--quiet)]">No runs yet.</p>
            )}
          </SettingsSection>

          {earlierRuns.length > 0 && (
            <SettingsSection title="Earlier runs">
              <div className="divide-y divide-[var(--line)]">
                {earlierRuns.map((entry) => (
                  <details key={entry.id} className="group">
                    <summary className="flex min-h-9 cursor-pointer items-center gap-2 px-4 text-[12px] text-[var(--muted)] hover:text-[var(--text)]">
                      <span
                        className={`capitalize ${entry.status === "failed" ? "text-[var(--danger)]" : "text-[var(--muted)]"}`}
                      >
                        {entry.status}
                      </span>
                      <span className="text-[var(--quiet)]">{dateTime(entry.startedAt)}</span>
                      <span className="ml-auto text-[11px] text-[var(--quiet)]">
                        {entry.added + entry.updated + entry.removed > 0
                          ? `${entry.added + entry.updated + entry.removed} changes`
                          : "No change"}
                      </span>
                    </summary>
                    <TasteRunReport run={entry} />
                  </details>
                ))}
              </div>
            </SettingsSection>
          )}
        </>
      )}
    </>
  );
}
