import type { Notify } from "../../../features/notifications/types";
import type { SpeechStage } from "../settingsPages";
import { FlashHighlight, Segmented, SettingsPageTitle } from "../../SettingsSection";
import { SpeechToTextList } from "./SpeechToTextList";
import { VoiceRecognitionPage } from "./VoiceRecognitionPage";
import { WakeWordPage } from "./WakeWordPage";

/**
 * The whole speech pipeline, on one page.
 *
 * Every stage is a question about the same microphone, and each is optional:
 * transcription is the only one that has to be on, and pressing the button
 * always works whether or not the two gates above it are enabled. The strip sits
 * in the title band beside the heading, the way every other page puts its one
 * control there, and lists the stages in the order audio passes through them.
 *
 * Which stage is showing is held by the settings window rather than here: the
 * page unmounts whenever a drill-down or another section is opened, so a stage
 * kept locally is forgotten by exactly the downloads and settings pages that are
 * reached from it.
 */
const STAGES: readonly SpeechStage[] = ["Voice recognition", "Wake word", "Speech to text"] as const;

export function SpeechToTextPage({
  stage,
  onStageChange,
  onDownload,
  onAudioDownload,
  onModelSettings,
  onWakeModelSettings,
  onDownloadWake,
  onCreateVoice,
  notify,
}: {
  stage: SpeechStage;
  onStageChange: (stage: SpeechStage) => void;
  onDownload: () => void;
  onAudioDownload: () => void;
  onModelSettings: (model: string) => void;
  /** A wake pack's options page, which returns to this stage rather than to the audio models list. */
  onWakeModelSettings: (model: string) => void;
  onDownloadWake: () => void;
  onCreateVoice: () => void;
  notify: Notify;
}) {
  return (
    <>
      <SettingsPageTitle>
        <h2 className="text-[17px] font-semibold tracking-tight">
          <FlashHighlight anchor="stt.engine" />
          Speech to text
        </h2>
        <Segmented label="Stage" value={stage} options={STAGES} onChange={onStageChange} />
      </SettingsPageTitle>

      {stage === "Speech to text" ? (
        <SpeechToTextList
          onDownload={onDownload}
          onAudioDownload={onAudioDownload}
          onModelSettings={onModelSettings}
          notify={notify}
        />
      ) : stage === "Wake word" ? (
        <WakeWordPage
          notify={notify}
          onDownload={onDownloadWake}
          onModelSettings={onWakeModelSettings}
        />
      ) : (
        <VoiceRecognitionPage notify={notify} onCreate={onCreateVoice} />
      )}
    </>
  );
}
