import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import { FlashHighlight, SettingsPageTitle, SettingRow, SettingsSection, Toggle } from "../../SettingsSection";
import type { Notify } from "../../../features/notifications/types";
import type { TtsSettings } from "../../../features/tts/types";
import { SpeechSynthesisList } from "./SpeechSynthesisList";

/**
 * The Text to speech page, under the Voice section.
 *
 * The voice list is the page's body and the settings that are not about one
 * voice sit under it as ordinary rows -- one section, no tab strip. The skip
 * code switch is here because it applies to every voice; a voice's own page
 * keeps only what is that voice's (its pace), reached by the gear beside it.
 */
export function TextToSpeechPage({
  onDownload,
  onVoiceSettings,
  notify,
}: {
  onDownload: () => void;
  onVoiceSettings: (voice: string) => void;
  notify: Notify;
}) {
  const [settings, setSettings] = useState<TtsSettings | null>(null);

  // Read on mount only: the list component below owns its own reads, and a
  // save cannot fail and revisit this record except through this page.
  useEffect(() => {
    let live = true;
    void invoke<TtsSettings>("tts_settings")
      .then((stored) => {
        if (live) setSettings(stored);
      })
      .catch((reason: unknown) =>
        notify("error", `Could not read the speech settings: ${String(reason)}`),
      );
    return () => {
      live = false;
    };
  }, [notify]);

  /** Painted first, then stored: the switch follows the press, and a failing
      save is reported rather than silently undone. */
  const setSkipCode = async (next: boolean) => {
    if (settings === null) return;
    const previous = settings;
    setSettings({ ...settings, skipCode: next });
    try {
      await invoke("tts_set_skip_code", { skip: next });
    } catch (reason) {
      setSettings(previous);
      notify("error", `That could not be saved: ${String(reason)}`);
    }
  };

  return (
    <>
      <SettingsPageTitle>
        <h2 className="text-[17px] font-semibold tracking-tight">
          <FlashHighlight anchor="tts.voices" />
          Text to speech
        </h2>
      </SettingsPageTitle>
      <SpeechSynthesisList onDownload={onDownload} onVoiceSettings={onVoiceSettings} notify={notify} />

      <SettingsSection title="Reading replies">
        <SettingRow
          anchor="tts.skipCode"
          label="Skip code blocks"
          description="Code in a reply is left out of what is spoken. Inline code too."
          control={
            settings !== null && (
              <Toggle
                checked={settings.skipCode}
                onChange={(next) => void setSkipCode(next)}
                label="Skip code blocks"
              />
            )
          }
        />
      </SettingsSection>
    </>
  );
}
