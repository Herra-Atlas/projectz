import { SettingsPageTitle, SettingRow, SettingsSection } from "../../SettingsSection";
import SingleModelPicker from "../../ModelSelectionPicker";
import TitleModelPicker from "../../TitleModelPicker";
import { MAX_MESSAGING_MODELS, type Preferences } from "../../../features/models/usePreferences";
import { NumberInput, ReasoningSegmented, type ModelPageProps } from "./shared";

/**
 * The Model section's "Main model" page.
 *
 * What a new conversation opens on, how hard it reasons and how far it may go
 * — the four rows a reader tunes *before* anything about the conversation
 * itself is decided, which is why this is the section's first page.
 *
 * **What is not here anywhere else also lives here on purpose:** the messaging
 * chain is a main-model question too — "which model answers when I am not
 * looking" — and keeping it beside the picker that opens a conversation means
 * both answers to "what do new things use" sit under one heading.
 */
export default function MainPage({ preferences, onChange, groups, localModels }: ModelPageProps) {
  const set = (field: Partial<Preferences>) => onChange({ ...preferences, ...field });
  return (
    <div>
      <SettingsPageTitle>
        <h2 className="text-[17px] font-semibold tracking-tight">Main model</h2>
      </SettingsPageTitle>

      <SettingsSection title="Model">
        {/* Not a lock: the model last switched to in a chat still wins. This row
            is where a *new* conversation starts after everything else has had
            its say, which is why the empty state says "Last used" rather than
            "none" — nothing chosen is a real state, not a hole. */}
        <SettingRow
          anchor="mainModel.model"
          label="Main model"
          description="What new conversations open on. The model you last switched to in a chat still wins."
          control={
            <SingleModelPicker
              value={preferences.mainModel}
              onChange={(mainModel) => set({ mainModel })}
              groups={groups}
              localModels={localModels}
              emptyLabel="Last used"
              label="Main model"
            />
          }
        />
        <SettingRow
          anchor="mainModel.reasoning"
          label="Reasoning"
          description="Default reasoning effort for a new conversation. Sent verbatim to the provider."
          control={
            <ReasoningSegmented
              label="Reasoning effort"
              value={preferences.mainReasoning}
              onChange={(mainReasoning) => set({ mainReasoning })}
            />
          }
        />
        {/* `0` is not "small": it is "use whatever the provider or the GGUF
            header detected". The row's default state therefore has to read as a
            real value too, which is why the input is never left blank. */}
        <SettingRow
          anchor="mainModel.context"
          label="Context window override"
          description="Forces this context window for the main model, ignoring whatever was detected. 0 uses the model's detected value."
          control={
            <NumberInput
              value={preferences.mainContextWindow}
              min={0}
              label="Context window override in tokens"
              onChange={(mainContextWindow) => set({ mainContextWindow })}
            />
          }
        />
      </SettingsSection>

      <SettingsSection title="Messaging">
        {/* The title picker drives this row rather than the single one because it
            is a chain, not a choice: a provider being down costs a retry, not
            the feature. The limit is the messaging chain's own, not the title
            chain's — the two happen to agree at three, but one is a rule about
            titles and this is a rule about messaging. */}
        <TitleModelPicker
          anchor="mainModel.messaging"
          models={preferences.messagingModels}
          onChange={(messagingModels) => set({ messagingModels })}
          groups={groups}
          localModels={localModels}
          max={MAX_MESSAGING_MODELS}
          label="Messaging"
          description="Answer a message with the first model that can. A model that fails is tried again, then the next one here."
        />
      </SettingsSection>

      <SettingsSection title="Limits">
        <SettingRow
          anchor="mainModel.steps"
          label="Step limit"
          description="How many tool rounds a reply may use before it is told to stop and answer in prose."
          control={
            <NumberInput
              value={preferences.mainSteps}
              min={1}
              label="Step limit in tool rounds"
              onChange={(mainSteps) => set({ mainSteps })}
            />
          }
        />
      </SettingsSection>
    </div>
  );
}
