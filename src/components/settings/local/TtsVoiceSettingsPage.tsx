import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { LoaderCircle, Play } from "lucide-react";

import { SettingsPageTitle } from "../../SettingsSection";
import { formatBytes } from "../../../features/downloads/format";
import type { Notify } from "../../../features/notifications/types";
import type { VoiceEntry, VoiceSpeed } from "../../../features/tts/types";
import { useVoiceTest } from "../../../features/tts/useVoiceTest";

/** How long the speed slider has to rest before it is stored. */
const SAVE_AFTER_MS = 250;

/**
 * One voice's settings.
 *
 * The pace is the setting, because it is the only thing Piper exposes per voice
 * that a reader would want to change -- the model itself is the file. `voice` is
 * `null` for the one the build placed, which is the same value the selection uses
 * for it, so the two pages cannot disagree about which voice a pace belongs to.
 */
export function TtsVoiceSettingsPage({
  voice,
  notify,
}: {
  voice: string | null;
  notify: Notify;
}) {
  const [entry, setEntry] = useState<VoiceEntry | null>(null);
  const [speed, setSpeed] = useState<VoiceSpeed | null>(null);
  const speedSave = useRef<number | null>(null);
  const { testing, test } = useVoiceTest((message) => notify("error", `Could not speak: ${message}`));

  const load = useCallback(async () => {
    try {
      const [list, pace] = await Promise.all([
        invoke<VoiceEntry[]>("tts_voices"),
        invoke<VoiceSpeed>("tts_speed", { voice }),
      ]);
      setSpeed(pace);
      setEntry(
        (voice === null ? list.find((item) => item.builtIn) : list.find((item) => item.id === voice)) ?? null,
      );
    } catch (reason) {
      notify("error", `Could not read this voice: ${String(reason)}`);
    }
  }, [notify, voice]);

  useEffect(() => {
    void load();
    return () => {
      if (speedSave.current !== null) window.clearTimeout(speedSave.current);
    };
  }, [load]);

  const changeSpeed = (value: number) => {
    setSpeed((current) => (current === null ? current : { ...current, speed: value }));
    if (speedSave.current !== null) window.clearTimeout(speedSave.current);
    speedSave.current = window.setTimeout(() => {
      void invoke("tts_set_speed", { voice, speed: value }).catch((reason: unknown) =>
        notify("error", `The speed could not be saved: ${String(reason)}`),
      );
    }, SAVE_AFTER_MS);
  };

  const title = entry?.name ?? (voice === null ? "Built-in voice" : "Voice");

  return (
    <>
      <SettingsPageTitle>
        <h2 className="text-[17px] font-semibold tracking-tight">{title}</h2>
        {/* The facts sit beside the name rather than in a row of their own: they
            describe the voice, and a whole line for two numbers pushed the only
            control on the page below the fold of a small window. */}
        {entry !== null && (
          <span className="text-xs text-[var(--quiet)]">
            {(entry.sampleRate / 1000).toFixed(1)} kHz · {formatBytes(entry.size)}
            {entry.builtIn ? " · built in" : ""}
          </span>
        )}
      </SettingsPageTitle>

      <div className="divide-y divide-[var(--line)]">
        <section className="px-4 py-3">
          <div className="flex items-baseline justify-between gap-3">
            <h3 className="text-sm font-medium">Speed</h3>
            <span className="text-xs tabular-nums text-[var(--muted)]">
              {(speed?.speed ?? 1).toFixed(2)}×
            </span>
          </div>
          <p className="mt-0.5 text-xs text-[var(--quiet)]">
            Applies to this voice only; the other voices keep their own.
          </p>
          <input
            type="range"
            min={speed?.minSpeed ?? 0.5}
            max={speed?.maxSpeed ?? 2}
            step={0.05}
            value={speed?.speed ?? 1}
            disabled={speed === null}
            onChange={(event) => changeSpeed(Number(event.target.value))}
            aria-label={`Speed of ${title}`}
            className="mt-2 w-full accent-[var(--accent)] disabled:opacity-40"
          />
          <button
            type="button"
            onClick={() => void test(voice)}
            disabled={testing || entry === null}
            className="mt-2 inline-flex min-h-8 items-center gap-1.5 rounded-md border border-[var(--line)] px-2.5 text-[12px] text-[var(--text)] transition-colors hover:border-[var(--line-strong)] hover:bg-[var(--raised)] disabled:opacity-40 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
          >
            {testing ? <LoaderCircle size={13} className="animate-spin" /> : <Play size={13} />}
            <span>{testing ? "Speaking…" : "Test voice"}</span>
          </button>
        </section>
      </div>
    </>
  );
}
