import { useState } from "react";
import { ArrowLeft } from "lucide-react";

import type { Notify } from "../../../features/notifications/types";
import { useVoices } from "../../../features/voice/useVoices";
import { SettingRow, SettingsPageTitle, SettingsSection } from "../../SettingsSection";

/**
 * Creating one voice: prompted takes, then a name.
 *
 * The sentences are fixed and different on purpose. A voiceprint built from one
 * repeated phrase learns *that phrase* as much as the person behind it, and the
 * model compares whole utterances -- so variety here is what makes a later
 * "hey dust" match at all.
 */
const PHRASES = [
  "The quick brown fox jumps over the lazy dog.",
  "Please close the window before the room gets cold.",
  "I am recording my voice so this app knows it is me.",
];

export function VoiceEnrollPage({ onBack, notify }: { onBack: () => void; notify: Notify }) {
  const { status, recordTake, saveVoice, discard } = useVoices((message) =>
    notify("error", message),
  );
  const [take, setTake] = useState(0);
  const [recording, setRecording] = useState(false);
  const [name, setName] = useState("");

  if (!status) return null;

  const done = take >= PHRASES.length;
  const held = status.pending;

  const record = async () => {
    if (recording || done) return;
    setRecording(true);
    const accepted = await recordTake();
    setRecording(false);
    if (accepted) setTake((current) => current + 1);
  };

  const finish = () => {
    void saveVoice(name.trim()).then((next) => {
      if (next) onBack();
    });
  };

  const cancel = () => {
    void discard().then(onBack);
  };

  return (
    <>
      <div className="mb-2 flex items-center gap-2 px-1">
        <button
          type="button"
          onClick={cancel}
          aria-label="Back"
          className="grid size-7 place-items-center rounded-md text-[var(--quiet)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
        >
          <ArrowLeft size={14} />
        </button>
        <span className="text-[11px] text-[var(--quiet)]">
          {done ? "Name it" : `Take ${take + 1} of ${PHRASES.length}`}
        </span>
      </div>
      <SettingsPageTitle>
        <h2>Create a voice</h2>
      </SettingsPageTitle>

      {done ? (
        <SettingsSection title="Saved as">
          <SettingRow
            anchor="voiceid.save"
            label="Name"
            description={`${held} ${held === 1 ? "take" : "takes"} ready. This is what a recognised voice is called in a prompt.`}
            control={
              <input
                value={name}
                onChange={(event) => setName(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter" && name.trim() && held > 0) finish();
                }}
                placeholder="e.g. Sam"
                aria-label="Voice name"
                autoFocus
                className="h-9 w-48 shrink-0 rounded-md border border-[var(--line)] bg-[var(--rail)] px-2 text-xs text-[var(--text)] placeholder:text-[var(--quiet)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
              />
            }
          />
          <SettingRow
            anchor="voiceid.saveVoice"
            label="Save voice"
            description="Only the averaged vector is stored; the takes are discarded."
            control={
              <div className="flex shrink-0 items-center gap-2">
                <button
                  type="button"
                  onClick={cancel}
                  className="rounded-md px-2 py-1 text-[11px] text-[var(--quiet)] transition-colors hover:text-[var(--text)]"
                >
                  Discard
                </button>
                <button
                  type="button"
                  onClick={finish}
                  disabled={!name.trim() || held === 0}
                  className="rounded-md border border-[var(--line)] px-2.5 py-1 text-[11px] font-medium text-[var(--text)] transition-colors hover:border-[var(--accent)] hover:text-[var(--accent)] disabled:opacity-40"
                >
                  Save
                </button>
              </div>
            }
          />
        </SettingsSection>
      ) : (
        <SettingsSection title="Say this">
          <div className="px-4 py-8">
            <p className="text-center text-lg leading-relaxed text-[var(--text)]">
              “{PHRASES[take]}”
            </p>
          </div>
          <SettingRow
            anchor="voiceid.take"
            label={recording ? "Listening…" : take === 0 ? "Start" : "Next take"}
            description={
              recording
                ? "Say the sentence, then stop. It ends itself on silence."
                : held === 0
                  ? "Three takes, a sentence each. Vary how you say them."
                  : `${held} ${held === 1 ? "take" : "takes"} held.`
            }
            control={
              <div className="flex shrink-0 items-center gap-2">
                <button
                  type="button"
                  onClick={cancel}
                  className="rounded-md px-2 py-1 text-[11px] text-[var(--quiet)] transition-colors hover:text-[var(--text)]"
                >
                  Cancel
                </button>
                <button
                  type="button"
                  onClick={() => void record()}
                  disabled={recording}
                  className="rounded-md border border-[var(--line)] px-2.5 py-1 text-[11px] font-medium text-[var(--text)] transition-colors hover:border-[var(--accent)] hover:text-[var(--accent)] disabled:opacity-40"
                >
                  {recording ? "Listening…" : take === 0 ? "Start" : "Record"}
                </button>
              </div>
            }
          />
        </SettingsSection>
      )}
    </>
  );
}
