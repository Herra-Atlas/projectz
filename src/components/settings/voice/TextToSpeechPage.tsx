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
  /** The download page each engine's voices install from. */
  onDownload: (engine: "tts" | "audio") => void;
  /** One voice's settings, named by its model path and the engine that runs it. */
  onVoiceSettings: (voice: string, engine: string) => void;
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
  const flip = async (
    field: "skipCode" | "skipMarkdown" | "autoSpeak" | "autoSpeakOnlyLoaded",
    next: boolean,
    command: string,
  ) => {
    if (settings === null) return;
    const previous = settings;
    setSettings({ ...settings, [field]: next });
    // The two skip commands name their argument `skip`, the auto-speak ones `on`.
    const args = field.startsWith("auto") ? { on: next } : { skip: next };
    try {
      await invoke(command, args);
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
                onChange={(next) => void flip("skipCode", next, "tts_set_skip_code")}
                label="Skip code blocks"
              />
            )
          }
        />
        <SettingRow
          anchor="tts.skipMarkdown"
          label="Skip markdown syntax"
          description="Symbols like **, # and [links] are left out, so the voice reads what you see — not what wrote it."
          control={
            settings !== null && (
              <Toggle
                checked={settings.skipMarkdown}
                onChange={(next) => void flip("skipMarkdown", next, "tts_set_skip_markdown")}
                label="Skip markdown syntax"
              />
            )
          }
        />
        <SettingRow
          anchor="tts.autoSpeak"
          label="Auto speak next messages"
          description="The chosen voice reads each new reply as it finishes, without being pressed."
          control={
            settings !== null && (
              <Toggle
                checked={settings.autoSpeak}
                onChange={(next) => void flip("autoSpeak", next, "tts_set_auto_speak")}
                label="Auto speak next messages"
              />
            )
          }
        />
        {settings?.autoSpeak && (
          <SettingRow
            anchor="tts.autoSpeakOnlyLoaded"
            label="Only if the voice is already ready"
            description="Off, a cold voice is started and loaded the moment a reply finishes. On, the reply waits unspoken until the voice is warm."
            control={
              <Toggle
                checked={settings.autoSpeakOnlyLoaded}
                onChange={(next) => void flip("autoSpeakOnlyLoaded", next, "tts_set_auto_speak_only_loaded")}
                label="Only if the voice is already ready"
              />
            }
          />
        )}
      </SettingsSection>
    </>
  );
}
