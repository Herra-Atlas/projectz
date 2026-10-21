import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import { SettingsPageTitle, Toggle } from "../../SettingsSection";
import type { Notify } from "../../../features/notifications/types";
import type { TtsSettings } from "../../../features/tts/types";

/**
 * The speech synthesis settings that apply to every voice.
 *
 * The page used to hold the voice list and the pace as well. Those are properties
 * of one voice, so they live on that voice's own page -- and what is left here is
 * the one decision that is not about a voice at all: whether the code in a reply
 * is read out with the prose.
 */
export function TtsSettingsPage({ notify }: { notify: Notify }) {
  const [settings, setSettings] = useState<TtsSettings | null>(null);

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

  const setSkipCode = async (next: boolean) => {
    if (settings === null) return;
    const previous = settings;
    // Painted first, then stored: the switch should follow the press, and a save
    // that fails is reported rather than silently undone.
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
        <h2 className="text-[17px] font-semibold tracking-tight">Speech synthesis</h2>
      </SettingsPageTitle>

      <section className="flex items-start justify-between gap-4 px-4 py-3">
        <div className="min-w-0">
          <h3 className="text-sm font-medium">Skip code blocks</h3>
          <p className="mt-0.5 text-xs leading-5 text-[var(--quiet)]">
            Code in a reply is left out of what is spoken. Inline code too.
          </p>
        </div>
        {settings !== null && (
          <Toggle
            checked={settings.skipCode}
            onChange={(next) => void setSkipCode(next)}
            label="Skip code blocks"
          />
        )}
      </section>
    </>
  );
}
