import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open as openFile } from "@tauri-apps/plugin-dialog";
import { Check, Cpu, Settings2, Trash2, Volume2 } from "lucide-react";

import { formatBytes } from "../../../features/downloads/format";
import type { Notify } from "../../../features/notifications/types";
import type { TtsSettings, VoiceEntry } from "../../../features/tts/types";
import { LocalAddMenu } from "./LocalAddMenu";

/**
 * The synthesis half of the Local page.
 *
 * Shaped like the llama.cpp list -- an engine, then what is installed on it --
 * because that is the question this page answers. A Piper voice is not a model
 * with settings to tune, so the row carries what the voice *is* (its rate, its
 * size) and the only inline action is choosing it.
 *
 * Choosing is inline rather than behind a page: the list is the picker, and a
 * picker that has to be opened to pick is one more step for the most common thing
 * done here. The tick says which one is in use without tinting the row -- a
 * filled row reads as selected *and* hovered, and this list has both.
 */
export function SpeechSynthesisCategory({
  onDownload,
  onSettings,
  onVoiceSettings,
  notify,
}: {
  onDownload: () => void;
  /** The engine's own settings. */
  onSettings: () => void;
  /** One voice's settings, named by its model path. */
  onVoiceSettings: (voice: string) => void;
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

  const addVoice = async () => {
    const picked = await openFile({
      multiple: false,
      filters: [{ name: "Piper voice", extensions: ["onnx"] }],
    });
    if (typeof picked !== "string") return;
    try {
      const added = await invoke<VoiceEntry>("tts_add_voice", { path: picked });
      await refresh();
      notify("success", `${added.name} added`);
    } catch (reason) {
      notify("error", `The voice could not be added: ${String(reason)}`);
    }
  };

  const removeVoice = async (voice: VoiceEntry) => {
    try {
      await invoke("tts_remove_voice", { voice: voice.id });
      // Re-read both: deleting the selected voice moves the selection back to the
      // built-in one, and the tick has to show where it actually went.
      await refresh();
      notify("success", `${voice.name} deleted`);
    } catch (reason) {
      notify("error", `The voice could not be deleted: ${String(reason)}`);
    }
  };

  const selected = settings?.voice ?? null;

  return (
    <article>
      <div className="flex min-h-[68px] items-center gap-2 px-4 py-3">
        <span className="grid size-9 shrink-0 place-items-center rounded-md text-[var(--muted)]">
          <Cpu size={16} />
        </span>
        <div className="min-w-0 flex-1">
          <h3 className="truncate text-sm font-medium">Piper</h3>
          <p className="truncate text-xs text-[var(--quiet)]">
            {voices === null
              ? "Checking…"
              : voices.length === 0
                ? "No voices installed"
                : voices.length === 1
                  ? "1 voice"
                  : `${voices.length} voices`}
          </p>
        </div>
        <LocalAddMenu
          anchor="local.tts.add"
          label="Add voice"
          downloadLabel="Download voice"
          addLabel="Add voice"
          onDownload={onDownload}
          onAdd={() => void addVoice()}
        />
        <button
          type="button"
          onClick={onSettings}
          aria-label="Speech synthesis settings"
          title="Speech synthesis settings"
          className="grid size-8 shrink-0 place-items-center rounded-md border border-[var(--line)] text-[var(--muted)] transition-colors hover:border-[var(--line-strong)] hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
        >
          <Settings2 size={14} />
        </button>
      </div>

      <div className="mb-3 ml-[2.75rem] border-l border-[var(--line)] py-1 pl-3">
        {voices?.map((voice) => {
          const isChosen = selected === voice.id;
          return (
            <div key={voice.id} className="flex items-center gap-1">
              <button
                type="button"
                onClick={() => void choose(voice)}
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
                    {(voice.sampleRate / 1000).toFixed(1)} kHz · {formatBytes(voice.size)}
                  </p>
                </div>
                {isChosen && <Check size={14} className="shrink-0 text-[var(--accent)]" />}
              </button>
              {/* Beside the row rather than inside it: a press on the row means
                  "use this voice", and a press on the gear means "change it".
                  Two buttons in one target is how both become unreliable. */}
              <button
                type="button"
                onClick={() => onVoiceSettings(voice.id)}
                aria-label={`${voice.name} settings`}
                title={`${voice.name} settings`}
                className="grid size-8 shrink-0 place-items-center rounded-md text-[var(--quiet)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
              >
                <Settings2 size={14} />
              </button>
              <button
                type="button"
                onClick={() => void removeVoice(voice)}
                aria-label={`Delete ${voice.name}`}
                title="Delete voice"
                className="grid size-8 shrink-0 place-items-center rounded-md text-[var(--quiet)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--danger)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
              >
                <Trash2 size={14} />
              </button>
            </div>
          );
        })}

        {/* Only when there is nothing at all: a voice added by hand is exactly what
            this list is missing, so the sentence names that as the way out. */}
        {voices?.length === 0 && (
          <p className="py-2 text-xs text-[var(--quiet)]">
            No voices found. Download one, or add an .onnx voice you already have.
          </p>
        )}
      </div>
    </article>
  );
}
