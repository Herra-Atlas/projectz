import { useState } from "react";
import { Trash2 } from "lucide-react";

import type { Notify } from "../../../features/notifications/types";
import { useVoices } from "../../../features/voice/useVoices";
import { SettingRow, SettingsSection, Toggle } from "../../SettingsSection";

/**
 * Who is speaking: the switch, the floor, and the enrolled voices.
 *
 * Creating a voice lives on its own page because it is a sequence -- three
 * prompted takes, then a name -- and a sequence does not belong in a row that
 * can only hold one press.
 */
export function VoiceRecognitionPage({
  notify,
  onCreate,
}: {
  notify: Notify;
  onCreate: () => void;
}) {
  const { status, setEnabled, setThreshold, remove } = useVoices((message) =>
    notify("error", message),
  );
  // The slider moves locally and is stored on release: a write per pointer move
  // would be a write per pixel.
  const [draft, setDraft] = useState<number | null>(null);

  if (!status) return null;

  const threshold = draft ?? status.threshold;
  const ready = status.installed && status.voices.length > 0;

  return (
    <>
      <SettingsSection title="Listening">
        <SettingRow
          anchor="voiceid.enabled"
          label="Recognise voices"
          description={
            !status.installed
              ? "Install Sherpa-onnx on the Audio engine page first."
              : status.voices.length === 0
                ? "Create a voice first."
                : status.enabled
                  ? "On. A wake word from a voice that is not enrolled is ignored."
                  : "Off. Any voice can start a dictation."
          }
          control={
            <Toggle
              checked={status.enabled}
              onChange={(next) => {
                // The command refuses both cases with a message; not asking is
                // quieter for a switch that cannot mean anything yet.
                if (!ready) return;
                void setEnabled(next);
              }}
              label="Recognise voices"
            />
          }
        />
        <SettingRow
          anchor="voiceid.threshold"
          label="How close a match"
          description={`${Math.round(threshold * 100)}% similarity. Raise it if someone else gets through; lower it if you are ignored.`}
          control={
            <input
              type="range"
              min={0.2}
              max={0.9}
              step={0.01}
              value={threshold}
              disabled={!ready}
              aria-label="How close a match"
              onChange={(event) => setDraft(Number(event.target.value))}
              onPointerUp={() => {
                if (draft === null) return;
                void setThreshold(draft).then(() => setDraft(null));
              }}
              onKeyUp={() => {
                if (draft === null) return;
                void setThreshold(draft).then(() => setDraft(null));
              }}
              className="w-40 accent-[var(--accent)]"
            />
          }
        />
      </SettingsSection>

      <SettingsSection title="Voices">
        {status.voices.length === 0 ? (
          <SettingRow
            anchor="voiceid.voices"
            label="Nothing enrolled"
            description="Create a voice to let this microphone answer to one person."
            control={null}
          />
        ) : (
          status.voices.map((voice) => (
            <SettingRow
              key={voice.name}
              anchor={`voiceid.${voice.name}`}
              label={voice.name}
              description={`${voice.clips} ${voice.clips === 1 ? "take" : "takes"}`}
              control={
                <button
                  type="button"
                  aria-label={`Delete ${voice.name}`}
                  title={`Delete ${voice.name}`}
                  onClick={() => void remove(voice.name)}
                  className="grid size-8 place-items-center rounded-md text-[var(--quiet)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--danger)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
                >
                  <Trash2 size={14} />
                </button>
              }
            />
          ))
        )}
        <SettingRow
          anchor="voiceid.create"
          label="Create a voice"
          description="Three prompted takes, then a name."
          control={
            <button
              type="button"
              onClick={onCreate}
              disabled={!status.installed}
              className="rounded-md border border-[var(--line)] px-2.5 py-1 text-[11px] font-medium text-[var(--text)] transition-colors hover:border-[var(--accent)] hover:text-[var(--accent)] disabled:opacity-40"
            >
              Create
            </button>
          }
        />
      </SettingsSection>
    </>
  );
}
