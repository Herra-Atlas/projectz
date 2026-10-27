import { useState } from "react";

import type { Notify } from "../../../features/notifications/types";
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
 */
const STAGES = ["Voice recognition", "Wake word", "Speech to text"] as const;
type Stage = (typeof STAGES)[number];

export function SpeechToTextPage({
  onDownload,
  onAudioDownload,
  onModelSettings,
  onWakeModelSettings,
  onDownloadWake,
  onCreateVoice,
  notify,
}: {
  onDownload: () => void;
  onAudioDownload: () => void;
  onModelSettings: (model: string) => void;
  /** A wake pack's options page, which returns to this stage rather than to the audio models list. */
  onWakeModelSettings: (model: string) => void;
  onDownloadWake: () => void;
  onCreateVoice: () => void;
  notify: Notify;
}) {
  // Transcription is the default because it is the only stage the app cannot do
  // without: the other two only decide *when* it listens.
  const [stage, setStage] = useState<Stage>("Speech to text");

  return (
    <>
      <SettingsPageTitle>
        <h2 className="text-[17px] font-semibold tracking-tight">
          <FlashHighlight anchor="stt.engine" />
          Speech to text
        </h2>
        <Segmented label="Stage" value={stage} options={STAGES} onChange={setStage} />
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
