import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import type { SttModel } from "../../../features/stt/types";
import type { Notify } from "../../../features/notifications/types";
import { SettingsPageTitle } from "../../SettingsSection";

/**
 * Languages worth naming in a picker.
 *
 * A short list rather than Whisper's ninety-nine: these are the ones a person
 * dictating into this app is likely to be speaking, and a list that fits on
 * screen beats one that has to be searched. Anything else can be detected.
 */
const LANGUAGES: ReadonlyArray<{ code: string; name: string }> = [
  { code: "en", name: "English" },
  { code: "fi", name: "Finnish" },
  { code: "sv", name: "Swedish" },
  { code: "et", name: "Estonian" },
  { code: "de", name: "German" },
  { code: "fr", name: "French" },
  { code: "es", name: "Spanish" },
  { code: "it", name: "Italian" },
  { code: "pt", name: "Portuguese" },
  { code: "nl", name: "Dutch" },
  { code: "pl", name: "Polish" },
  { code: "ru", name: "Russian" },
  { code: "ja", name: "Japanese" },
  { code: "zh", name: "Chinese" },
];

/**
 * One recognition model's settings.
 *
 * Language is per model because that is what it actually configures: the engine
 * passes it to the model it is about to read. Detection is a second pass over the
 * audio, so naming the language is faster and stops a short recording being read
 * as a language nobody in the room speaks.
 */
export function SttModelSettingsPage({
  model,
  notify,
}: {
  model: string;
  notify: Notify;
}) {
  const [entry, setEntry] = useState<SttModel | null>(null);
  const [language, setLanguage] = useState("");

  useEffect(() => {
    let live = true;
    void invoke<SttModel[]>("stt_models")
      .then((models) => {
        if (live) setEntry(models.find((candidate) => candidate.path === model) ?? null);
      })
      .catch(() => undefined);
    void invoke<string>("stt_model_language", { model })
      .then((code) => {
        if (live) setLanguage(code);
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, [model]);

  const choose = (code: string) => {
    const previous = language;
    setLanguage(code);
    void invoke("stt_set_language", { model, language: code }).catch((reason: unknown) => {
      setLanguage(previous);
      notify("error", `The language could not be saved: ${String(reason)}`);
    });
  };

  const parakeet = entry?.parakeet === true;

  return (
    <>
      <SettingsPageTitle>
        <h2 className="text-[17px] font-semibold tracking-tight">{entry?.name ?? "Recognition model"}</h2>
        {entry && (
          <span className="text-xs text-[var(--quiet)]">
            {parakeet ? "Parakeet" : "Whisper"} · {(entry.size / 1_048_576).toFixed(0)} MB
          </span>
        )}
      </SettingsPageTitle>

      <section className="px-4 py-3">
        <label htmlFor="stt-language" className="block text-sm font-medium text-[var(--text)]">
          Language
        </label>
        <p className="mt-0.5 text-xs leading-5 text-[var(--quiet)]">
          {parakeet
            ? "Parakeet works out the language itself, so there is nothing to set."
            : "What you speak into the microphone. Detecting it costs a second pass over the audio."}
        </p>
        <select
          id="stt-language"
          value={language}
          onChange={(event) => choose(event.target.value)}
          disabled={parakeet}
          className="mt-2 h-9 w-56 rounded-md border border-[var(--line)] bg-[var(--rail)] px-2 text-sm text-[var(--text)] outline-none focus-visible:border-[var(--accent)] disabled:opacity-40"
        >
          <option value="">Detect automatically</option>
          {LANGUAGES.map((option) => (
            <option key={option.code} value={option.code}>
              {option.name}
            </option>
          ))}
        </select>
      </section>
    </>
  );
}
