import { FlashHighlight, SettingsPageTitle } from "../../SettingsSection";
import type { Notify } from "../../../features/notifications/types";
import { SpeechToTextList } from "./SpeechToTextList";

/**
 * The Speech to text page, under the Voice section.
 *
 * One list holds everything the engine offers at a glance -- whether it is
 * installed, which models exist, which one transcribes -- and a model's own
 * settings stay beside its row behind the gear. The page carries no categories:
 * there is one engine, so the tab strip was a second door into the same room.
 */
export function SpeechToTextPage({
  onDownload,
  onModelSettings,
  notify,
}: {
  onDownload: () => void;
  onModelSettings: (model: string) => void;
  notify: Notify;
}) {
  return (
    <>
      <SettingsPageTitle>
        <h2 className="text-[17px] font-semibold tracking-tight">
          <FlashHighlight anchor="stt.engine" />
          Speech to text
        </h2>
      </SettingsPageTitle>
      <SpeechToTextList onDownload={onDownload} onModelSettings={onModelSettings} notify={notify} />
    </>
  );
}
