import { SettingsPageTitle, SettingRow, SettingsSection, Toggle } from "../../SettingsSection";
import type { Preferences } from "../../../features/models/usePreferences";
import { NumberInput, type ModelPageProps } from "./shared";

/**
 * The Model section's "Context" page.
 *
 * How a conversation that outgrows the model's window is kept inside it. The
 * switch leads and the numbers follow — literally: while auto-compression is
 * off, the numbers are drawn disabled and dimmed, because a pass that is off
 * reads none of them and a row that looked live would be lying about it.
 */
export default function ContextPage({ preferences, onChange }: ModelPageProps) {
  const set = (field: Partial<Preferences>) => onChange({ ...preferences, ...field });
  // One gate answer for the page: when it is off, every number row is disabled.
  const gating = !preferences.autoCompact;
  // Note shown beside the switch while its answer is "off", so the disabled
  // numbers below it explain themselves rather than reading as broken controls.
  const offNote = "A pass that is off ignores these numbers.";
  return (
    <div>
      <SettingsPageTitle>
        <h2 className="text-[17px] font-semibold tracking-tight">Context</h2>
      </SettingsPageTitle>

      <SettingsSection title="Auto-compression">
        <SettingRow
          anchor="context.autoCompact"
          label="Auto-compression"
          description="Old tool output and turns are elided — or summarised with a model — when the conversation approaches the model's window. The transcript keeps everything."
          control={
            <div className="shrink-0">
              <Toggle
                label="Auto-compression"
                checked={preferences.autoCompact}
                onChange={(autoCompact) => set({ autoCompact })}
              />
              {/* Only the off state needs a line: on, the numbers below speak
                  for themselves; off, they do not, and someone arriving at this
                  page to turn the feature off should see the switch was heard. */}
              {!preferences.autoCompact && (
                <p className="mt-1.5 max-w-[220px] text-[11px] leading-4 text-[var(--muted)]">
                  {offNote}
                </p>
              )}
            </div>
          }
        />
        <SettingRow
          anchor="context.compressAt"
          label="When to compress"
          description="Percent of the measured context window that triggers a pass."
          control={
            <NumberInput
              value={preferences.compactAtPct}
              min={5}
              max={99}
              disabled={gating}
              label="Percent of the context window that triggers compression"
              onChange={(compactAtPct) => set({ compactAtPct })}
            />
          }
        />
        <SettingRow
          anchor="context.target"
          label="Compression target"
          description="After a pass, aim for this fraction of the window (0.0-1.0). Drives how deep the pass goes, it is not a guarantee."
          control={
            // The fraction the pass aims at: typed freely, clamped to the
            // 0-1 range on blur, since 1.5 of a window is no aim at all.
            <NumberInput
              value={preferences.compactTarget}
              min={0}
              max={1}
              step={0.05}
              disabled={gating}
              label="Compression target as a fraction of the window"
              onChange={(compactTarget) => set({ compactTarget })}
            />
          }
        />
        {/* The search index keywords this row with "threshold" rather than the
            label -- what it bounds is what dawns on a user late, so the words
            they type stay "reach", "threshold", "older". */}
        <SettingRow
          anchor="context.reach"
          label="Compression reach"
          description="How much of the older end of the conversation may be rewritten, as a fraction (0-1). The protected tail always wins."
          control={
            <NumberInput
              value={preferences.compactReach}
              min={0}
              max={1}
              step={0.05}
              disabled={gating}
              label="Compression reach as a fraction of the conversation"
              onChange={(compactReach) => set({ compactReach })}
            />
          }
        />
      </SettingsSection>

      <SettingsSection title="Summarising">
        <SettingRow
          anchor="context.protect"
          label="Protect recent messages"
          description="The last N messages of a conversation are always sent exactly as they were."
          hint="The protection beats everything else on this page: however deep a pass may reach, it stops at the tail. A number of 0 protects nothing, which is a real setting for a reader who wants the window used to the last token — not a mistake the input refuses."
          control={
            <NumberInput
              value={preferences.compactProtect}
              min={0}
              disabled={gating}
              label="Number of recent messages protected from compression"
              onChange={(compactProtect) => set({ compactProtect })}
            />
          }
        />
        <SettingRow
          anchor="context.timeout"
          label="Compression model timeout"
          description="When a model is used to summarise compressed turns — e.g. a slow local one — the summary call is cancelled after this many seconds and the pass keeps elision only."
          hint="A summary that never arrives must cost no more than one that fails outright: the pass degrades to plain elision, which always finishes, rather than waiting on a model that is still warming up. The transcript is never blocked by it."
          control={
            <NumberInput
              value={preferences.compactTimeout}
              min={5}
              disabled={gating}
              label="Compression model timeout in seconds"
              onChange={(compactTimeout) => set({ compactTimeout })}
            />
          }
        />
      </SettingsSection>
    </div>
  );
}
