import { SettingsPageTitle, SettingRow, SettingsSection } from "../../SettingsSection";
import SingleModelPicker from "../../ModelSelectionPicker";
import type { Preferences } from "../../../features/models/usePreferences";
import { ReasoningSegmented, NumberInput, type ModelPageProps } from "./shared";

/**
 * The Model section's "Subagents" page.
 *
 * One page for the whole sub-agent runtime: what it runs on, how long it may
 * work, how many run at once, and how it reasons. They belong together because
 * a sub-agent is one request with a budget — four settings that all answer the
 * same question, "what does a delegated job look like" — and splitting them
 * across the other Model pages would leave a reader hunting for the parallel
 * ceiling under whichever page came to mind first.
 */
export default function SubagentsPage({ preferences, onChange, groups, localModels }: ModelPageProps) {
  const set = (field: Partial<Preferences>) => onChange({ ...preferences, ...field });
  return (
    <div>
      <SettingsPageTitle>
        <h2 className="text-[17px] font-semibold tracking-tight">Subagents</h2>
      </SettingsPageTitle>

      <SettingsSection title="Model">
        {/* A sub-agent is one request, so there is no fallback chain here — the
            parent names a model when it wants one, and this only fills the
            blanks. "Same as chat" is the real default rather than a hole. */}
        <SettingRow
          anchor="subagents.model"
          label="Sub-agent model"
          description="Defaults to whatever the chat is using."
          control={
            <SingleModelPicker
              value={preferences.subagentModel}
              onChange={(subagentModel) => set({ subagentModel })}
              groups={groups}
              localModels={localModels}
              emptyLabel="Same as chat"
              label="Sub-agent model"
            />
          }
        />
        <SettingRow
          anchor="subagents.reasoning"
          label="Reasoning effort"
          description="Sent to every sub-agent. Provider default keeps whatever the parent used."
          control={
            <ReasoningSegmented
              label="Sub-agent reasoning effort"
              value={preferences.subagentReasoning}
              onChange={(subagentReasoning) => set({ subagentReasoning })}
            />
          }
        />
      </SettingsSection>

      <SettingsSection title="Budget">
        {/* Both numbers are ceilings, not requests: the model spends as many
            tool rounds as the work needs until one of them stops it. Zero is a
            real value only for the parallel cap, which is why the two rows say
            so in their own words and the inputs clamp differently. */}
        <SettingRow
          anchor="subagents.steps"
          label="Step limit"
          description="How many tool rounds a sub-agent may spend."
          control={
            <NumberInput
              value={preferences.subagentSteps}
              min={1}
              label="Sub-agent step limit in tool rounds"
              onChange={(subagentSteps) => set({ subagentSteps })}
            />
          }
        />
        <SettingRow
          anchor="subagents.parallel"
          label="Parallel sub-agents"
          description="How many may run at once. 0 means no ceiling; spawning more than this is refused by name."
          control={
            <NumberInput
              value={preferences.subagentParallel}
              min={0}
              label="Parallel sub-agents at once"
              onChange={(subagentParallel) => set({ subagentParallel })}
            />
          }
        />
      </SettingsSection>
    </div>
  );
}
