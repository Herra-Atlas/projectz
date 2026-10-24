import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Check, Download, Settings2, Trash2, Volume2 } from "lucide-react";

import { formatBytes } from "../../../features/downloads/format";
import type { Notify } from "../../../features/notifications/types";
import type { TtsSettings, VoiceEntry } from "../../../features/tts/types";
import { HoldToDelete } from "../../HoldToDelete";
import { ANCHOR_ATTRIBUTE } from "../../SettingsSection";

/**
 * The synthesis half of the Text to speech page, split by engine.
 *
 * Piper and audio.cpp are separate headers, the way the local page draws each of
 * its engines: a voice's download page is per engine, and a list that hides
 * which engine a voice belongs to is what makes "download a voice" mean two
 * different pages. Piper's voices come from the voice repo (`.onnx` plus
 * configs); audio.cpp's come from `models/audio` (gguf packages a download or a
 * copy placed there).
 *
 * Choosing is inline rather than behind a page: the list is the picker, and a
 * picker that has to be opened to pick is one more step for the most common thing
 * done here. The tick says which one is in use without tinting the row -- a
 * filled row reads as selected *and* hovered, and this list has both.
 */
export function SpeechSynthesisList({
  onDownload,
  onVoiceSettings,
  notify,
}: {
  /** The download page each engine installs from. */
  onDownload: (engine: "tts" | "audio") => void;
  /** One voice's settings, named by its model path and the engine that runs it. */
  onVoiceSettings: (voice: string, engine: string) => void;
  notify: Notify;
}) {
  const [voices, setVoices] = useState<VoiceEntry[] | null>(null);
  const [settings, setSettings] = useState<TtsSettings | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [list, stored] = await Promise.all([
        invoke<VoiceEntry[]>("tts_voices"),
        invoke<TtsSettings>("tts_settings"),
      ]);
      setVoices(list);
      setSettings(stored);
    } catch (reason) {
      setVoices([]);
      notify("error", `Could not read the voices: ${String(reason)}`);
    }
  }, [notify]);

  // Re-read on mount: this component is replaced while another page is open, so
  // coming back from a download is a fresh mount rather than a stale list.
  useEffect(() => {
    void refresh();
  }, [refresh]);

  const choose = async (voice: VoiceEntry) => {
    if (settings === null) return;
    if (settings.voice === voice.id) return;
    const previous = settings;
    // Painted first, then stored: the tick should follow the press, and a failed
    // save is reported rather than silently undoing it.
    setSettings({ ...settings, voice: voice.id });
    try {
      await invoke("tts_select_voice", { voice: voice.id });
    } catch (reason) {
      setSettings(previous);
      notify("error", `The voice could not be saved: ${String(reason)}`);
    }
  };

  const removeVoice = async (voice: VoiceEntry) => {
    try {
      // The folder a voice sits under decides the cleanup command, the same way
      // the backend resolves it: a Piper voice carries a config beside the
      // model, an audio.cpp voice does not.
      await invoke(
        voice.engine === "audiocpp" ? "tts_remove_audio_voice" : "tts_remove_voice",
        { voice: voice.id },
      );
      // Re-read both: deleting the selected voice moves the selection back to the
      // built-in one, and the tick has to show where it actually went.
      await refresh();
      notify("success", `${voice.name} deleted`);
    } catch (reason) {
      notify("error", `The voice could not be deleted: ${String(reason)}`);
    }
  };

  const selected = settings?.voice ?? null;
  const piper = (voices ?? []).filter((voice) => voice.engine === "piper");
  const audiocpp = (voices ?? []).filter((voice) => voice.engine === "audiocpp");

  return (
    <article>
      <EngineHeader
        title="Piper"
        subtitle={
          voices === null ? (
            <span className="inline-block h-3 w-32 animate-pulse rounded bg-[var(--raised)]" />
          ) : `${piper.length === 0 ? "No voices" : piper.length === 1 ? "1 voice" : `${piper.length} voices`}`
        }
        download={
          <button
            type="button"
            onClick={() => onDownload("tts")}
            className="inline-flex min-h-8 shrink-0 items-center gap-1.5 rounded-md border border-[var(--line)] px-2.5 text-[12px] text-[var(--text)] transition-colors hover:border-[var(--line-strong)] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
          >
            <Download size={13} />
            <span>Download voice</span>
          </button>
        }
      />
      <VoiceList
        voices={piper}
        selected={selected}
        onChoose={choose}
        onRemove={removeVoice}
        onSettings={(voice) => onVoiceSettings(voice, "piper")}
        anchor="tts.voices"
        empty="No Piper voices yet. Download one, or add an .onnx voice you already have."
      />

      {/* audio.cpp's voices, which arrive through the same registry once the
          engine ships its own page's models. */}
      <EngineHeader
        title="audio.cpp"
        subtitle={
          voices === null ? (
            <span className="inline-block h-3 w-32 animate-pulse rounded bg-[var(--raised)]" />
          ) : `${audiocpp.length === 0 ? "No voices" : audiocpp.length === 1 ? "1 voice" : `${audiocpp.length} voices`}`
        }
        download={
          <button
            type="button"
            onClick={() => onDownload("audio")}
            className="inline-flex min-h-8 shrink-0 items-center gap-1.5 rounded-md border border-[var(--line)] px-2.5 text-[12px] text-[var(--text)] transition-colors hover:border-[var(--line-strong)] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
          >
            <Download size={13} />
            <span>Download voice</span>
          </button>
        }
      />
      <VoiceList
        voices={audiocpp}
        selected={selected}
        onChoose={choose}
        onRemove={removeVoice}
        onSettings={(voice) => onVoiceSettings(voice, "audiocpp")}
        anchor="tts.voicesAudiocpp"
        empty="No audio.cpp voices yet. Download one from the audio.cpp models page."
      />
    </article>
  );
}

/** One engine's header: name, count, the download that fills it. */
function EngineHeader({
  title,
  subtitle,
  download,
}: {
  title: string;
  subtitle: React.ReactNode;
  download: React.ReactNode;
}) {
  return (
    <div className="flex min-h-[60px] items-center gap-2 px-4 py-3">
      <span className="grid size-9 shrink-0 place-items-center rounded-md text-[var(--muted)]">
        <Volume2 size={16} />
      </span>
      <div className="min-w-0 flex-1">
        <h3 className="truncate text-sm font-medium">{title}</h3>
        <p className="truncate text-xs text-[var(--quiet)]">{subtitle}</p>
      </div>
      {download}
    </div>
  );
}

/** The voices of one engine, with the gear and the held delete. */
function VoiceList({
  voices,
  selected,
  onChoose,
  onRemove,
  onSettings,
  anchor,
  empty,
}: {
  voices: VoiceEntry[];
  selected: string | null;
  onChoose: (voice: VoiceEntry) => void;
  onRemove: (voice: VoiceEntry) => void;
  onSettings: (voice: string) => void;
  anchor: string;
  empty: string;
}) {
  return (
    <div className="mb-3 ml-[2.75rem] border-l border-[var(--line)] py-1 pl-3" {...{ [ANCHOR_ATTRIBUTE]: anchor }}>
      {voices.map((voice) => {
        const isChosen = selected === voice.id;
        return (
          <div key={voice.id} className="flex items-center gap-1">
            <button
              type="button"
              onClick={() => void onChoose(voice)}
              aria-pressed={isChosen}
              title={voice.id}
              className="flex min-h-9 min-w-0 flex-1 items-center gap-3 rounded-md py-1.5 pl-1 pr-2 text-left focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
            >
              <Volume2
                size={14}
                className={`shrink-0 ${isChosen ? "text-[var(--accent)]" : "text-[var(--quiet)]"}`}
              />
              <div className="min-w-0 flex-1">
                <h4 className="truncate text-xs font-medium text-[var(--muted)]">{voice.name}</h4>
                <p className="truncate text-[11px] text-[var(--quiet)]">
                  {/* `0` means the audio.cpp engine has not answered a synthesis
                      for this voice yet; showing a rate would be a guess. */}
                  {voice.sampleRate > 0 ? `${(voice.sampleRate / 1000).toFixed(1)} kHz · ` : ""}
                  {formatBytes(voice.size)}
                </p>
              </div>
              {isChosen && <Check size={14} className="shrink-0 text-[var(--accent)]" />}
            </button>
            {/* Beside the row rather than inside it: a press on the row means
                "use this voice", and a press on the gear means "change it".
                Two buttons in one target is how both become unreliable. */}
            <button
              type="button"
              onClick={() => onSettings(voice.id)}
              aria-label={`${voice.name} settings`}
              title={`${voice.name} settings`}
              className="grid size-8 shrink-0 place-items-center rounded-md text-[var(--quiet)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
            >
              <Settings2 size={14} />
            </button>
            <HoldToDelete
              onConfirm={() => void onRemove(voice)}
              label={`Delete ${voice.name}`}
              title="Delete voice"
              className="grid size-8 shrink-0 place-items-center rounded-md text-[var(--quiet)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--danger)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
            >
              <Trash2 size={14} />
            </HoldToDelete>
          </div>
        );
      })}
      {/* Nothing from this engine, while the other may carry some -- so the
          sentence is per section rather than page-wide. */}
      {voices.length === 0 && <p className="py-2 text-xs text-[var(--quiet)]">{empty}</p>}
    </div>
  );
}
