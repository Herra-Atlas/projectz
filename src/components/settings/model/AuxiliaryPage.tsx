import { SettingsPageTitle, SettingRow, SettingsSection } from "../../SettingsSection";
import SingleModelPicker from "../../ModelSelectionPicker";
import TitleModelPicker from "../../TitleModelPicker";
import { MAX_TITLE_MODELS, type Preferences } from "../../../features/models/usePreferences";
import type { ModelPageProps } from "./shared";

/**
 * The Model section's "Auxiliary models" page.
 *
 * The small jobs the assistant does around the conversation — naming it,
 * reading an image — rather than answering in it. They sit behind one page
 * because none of them is worth its own sidebar entry, and they are the two
 * settings a reader comes looking for exactly once: something misbehaved about
 * titles or an image now.
 */
export default function AuxiliaryPage({ preferences, onChange, groups, localModels }: ModelPageProps) {
  const set = (field: Partial<Preferences>) => onChange({ ...preferences, ...field });
  return (
    <div>
      <SettingsPageTitle>
        <h2 className="text-[17px] font-semibold tracking-tight">Auxiliary models</h2>
      </SettingsPageTitle>

      <SettingsSection title="Vision">
        {/* Off is a real answer, not an omission: without a vision model a
            failed read is the refusal by name — the image cannot be read —
            rather than a silent attempt against a model that cannot see. */}
        <SettingRow
          anchor="auxiliary.vision"
          label="Vision model"
          description="The model an image is shown to when a file is read."
          control={
            <SingleModelPicker
              value={preferences.visionModel}
              onChange={(visionModel) => set({ visionModel })}
              groups={groups}
              localModels={localModels}
              emptyLabel="Off"
              label="Vision model"
            />
          }
        />
      </SettingsSection>

      <SettingsSection title="Titles">
        {/* The title chain is the original ordered-choice picker; its own label
            describes it here, so the row reads with this page's words rather
            than the picker's defaults ("Title generation"). */}
        <TitleModelPicker
          anchor="auxiliary.title"
          models={preferences.sessionTitleModels}
          onChange={(sessionTitleModels) => set({ sessionTitleModels })}
          groups={groups}
          localModels={localModels}
          max={MAX_TITLE_MODELS}
          label="Conversation titles"
          description="Names a new conversation using the opening message. The first model that answers wins."
        />
        {/* No choice here either way — the vision picker is the single-select
            twin for the same reason in reverse: one image read has no fallback
            gate to thread, and a chain would only invite counting on it. */}
      </SettingsSection>
    </div>
  );
}
