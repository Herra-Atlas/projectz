import { useState, type ReactNode } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { AudioLines, Check, Pencil, Trash2, Waves } from "lucide-react";

import { formatBytes } from "../../../features/downloads/format";
import type { Notify } from "../../../features/notifications/types";
import type { WakeModel } from "../../../features/wake/types";
import { useWake } from "../../../features/wake/useWake";
import { HoldToDelete } from "../../HoldToDelete";
import { ANCHOR_ATTRIBUTE, SettingRow, SettingsSection, Toggle } from "../../SettingsSection";
import { LocalAddMenu } from "../local/LocalAddMenu";

/**
 * The wake word stage: one microphone held open, and the model that listens for it.
 *
 * Two engines answer the same question and keep their files in two places, so
 * the list is grouped by engine the way Speech to text groups recognition
 * models: **audio.cpp** runs the wake packs in `models/audio`, **openWakeWord**
 * runs the ONNX heads beside its own engine. A reader choosing a phrase needs
 * to know which runtime it will run on -- "wake word" said nothing about that.
 *
 * Models arrive three ways, and all of them are a file in a folder: **Download**
 * opens the downloader narrowed to the catalogue's wake families, **Add**
 * copies a trained `.gguf` or `.onnx` in from disk, and an audio.cpp model's
 * **edit** door opens its family's spec -- which is where a `sherpa_kws` model's
 * keyword rows live. There is no trainer here: a phrase of your own is trained
 * outside the app.
 */
export function WakeWordPage({
  notify,
  onDownload,
  onModelSettings,
}: {
  notify: Notify;
  onDownload: () => void;
  onModelSettings: (model: string) => void;
}) {
  const { status, setEnabled, selectModel, setThreshold, addModel, removeModel } = useWake(
    (message) => notify("error", message),
  );
  // The slider's own value until the drag ends: a change per pointer move would
  // be a write per pixel, and the bar would lag the finger while it saved.
  const [draft, setDraft] = useState<number | null>(null);

  const pick = async (extensions: string[]) => {
    const chosen = await open({
      multiple: false,
      filters: [{ name: "Wake word model", extensions }],
    });
    if (typeof chosen === "string") await addModel(chosen);
  };

  // Stored when the drag ends, so one gesture is one write and the bar follows
  // the pointer instead of the round trip.
  const commit = () => {
    if (draft === null) return;
    void setThreshold(draft).then(() => setDraft(null));
  };

  const models = status?.models ?? [];
  const packs = models.filter((model) => model.engine === "audio.cpp");
  const heads = models.filter((model) => model.engine === "openWakeWord");
  const anyEngine =
    status !== null && (status.audioEngineInstalled || status.openWakeWordInstalled);

  const detail =
    status === null
      ? "Reading…"
      : !anyEngine
        ? "No wake engine is installed. Install audio.cpp or openWakeWord from Audio engine."
        : models.length === 0
          ? "No wake word model is installed yet."
          : status.running
            ? "Listening now. The microphone stays open until this is switched off."
            : "Off. The microphone is closed, not ignored.";

  return (
    <>
      <EngineSection
        anchor="wake.audio"
        listAnchor="wake.models"
        addAnchor="wake.add"
        icon={<AudioLines size={16} />}
        title="audio.cpp"
        detail={
          status === null
            ? "Checking…"
            : status.audioEngineInstalled
              ? `${status.audioEngineVersion ?? "installed"} · ${count(packs.length)}`
              : "Engine not installed · install it from Audio engine"
        }
        models={packs}
        selected={status?.model ?? null}
        empty="No audio.cpp wake model yet. Download a package, or add a .gguf trained elsewhere."
        download={onDownload}
        addLabel="Add .gguf"
        onAdd={() => void pick(["gguf"])}
        onChoose={selectModel}
        onEdit={onModelSettings}
        onRemove={removeModel}
      />

      <EngineSection
        anchor="wake.openwakeword"
        listAnchor="wake.heads"
        addAnchor="wake.addHead"
        icon={<Waves size={16} />}
        title="openWakeWord"
        detail={
          status === null
            ? "Checking…"
            : status.openWakeWordInstalled
              ? `${status.openWakeWordVersion ?? "installed"} · ${count(heads.length)}`
              : "Engine not installed · install it from Audio engine"
        }
        models={heads}
        selected={status?.model ?? null}
        empty="No word heads yet. Download one from the collection, or add an .onnx head."
        download={onDownload}
        addLabel="Add .onnx"
        onAdd={() => void pick(["onnx"])}
        onChoose={selectModel}
        onEdit={onModelSettings}
        onRemove={removeModel}
      />

      <SettingsSection title="Listening">
        <SettingRow
          anchor="wake.enabled"
          label="Listen for a wake word"
          description={detail}
          hint="While this is on, the microphone is open continuously and every second of audio is sent to the wake model on this machine. Nothing leaves the machine, and switching it off closes the device."
          control={
            <Toggle
              checked={status?.enabled === true}
              onChange={(next) => void setEnabled(next)}
              label="Listen for a wake word"
            />
          }
        />
        <SettingRow
          anchor="wake.threshold"
          label="How sure it has to be"
          description={
            status === null ? "—" : `${Math.round((draft ?? status.threshold) * 100)}% confidence`
          }
          hint="The floor a detection has to clear. Higher is quieter and misses more; lower fires on less. A model that fires on the television is one whose floor is too low. This floor is sent to the families that name a threshold; a family with its own score names carries them on its edit page."
          control={
            <input
              type="range"
              min={0}
              max={1}
              step={0.05}
              value={draft ?? status?.threshold ?? 0.5}
              disabled={status === null}
              onChange={(event) => setDraft(Number(event.target.value))}
              onPointerUp={commit}
              onKeyUp={commit}
              onBlur={commit}
              aria-label="Wake word confidence threshold"
              className="w-[10rem] accent-[var(--accent)]"
            />
          }
        />
      </SettingsSection>
    </>
  );
}

/** One engine's card: its name, what it holds, and its own add control. */
function EngineSection({
  anchor,
  listAnchor,
  addAnchor,
  icon,
  title,
  detail,
  models,
  selected,
  empty,
  download,
  addLabel,
  onAdd,
  onChoose,
  onEdit,
  onRemove,
}: {
  anchor: string;
  listAnchor: string;
  addAnchor: string;
  icon: ReactNode;
  title: string;
  detail: string;
  models: WakeModel[];
  selected: string | null;
  empty: string;
  download: () => void;
  addLabel: string;
  onAdd: () => void;
  onChoose: (path: string) => Promise<void>;
  onEdit: (path: string) => void;
  onRemove: (path: string) => Promise<void>;
}) {
  return (
    <article>
      <div
        className="flex min-h-[68px] items-center gap-2 px-4 py-3"
        {...{ [ANCHOR_ATTRIBUTE]: anchor }}
      >
        <span className="grid size-9 shrink-0 place-items-center rounded-md text-[var(--muted)]">
          {icon}
        </span>
        <div className="min-w-0 flex-1">
          <h3 className="truncate text-sm font-medium">{title}</h3>
          <p className="truncate text-xs text-[var(--quiet)]">{detail}</p>
        </div>
        <LocalAddMenu
          anchor={addAnchor}
          onDownload={download}
          onAdd={onAdd}
          addLabel={addLabel}
        />
      </div>

      <div
        className="mb-3 ml-[2.75rem] border-l border-[var(--line)] py-1 pl-3"
        {...{ [ANCHOR_ATTRIBUTE]: listAnchor }}
      >
        {models.map((model) => (
          <WakeRow
            key={model.path}
            model={model}
            active={model.path === selected}
            onChoose={onChoose}
            onEdit={onEdit}
            onRemove={onRemove}
          />
        ))}
        {models.length === 0 && (
          <p className="py-2 text-xs text-[var(--quiet)]">{empty}</p>
        )}
      </div>
    </article>
  );
}

/** One model row: choose, edit its family's options, and a held delete. */
function WakeRow({
  model,
  active,
  onChoose,
  onEdit,
  onRemove,
}: {
  model: WakeModel;
  active: boolean;
  onChoose: (path: string) => Promise<void>;
  onEdit: (path: string) => void;
  onRemove: (path: string) => Promise<void>;
}) {
  return (
    <div className="flex items-center gap-1">
      <button
        type="button"
        onClick={() => void onChoose(model.path)}
        aria-pressed={active}
        title={model.path}
        className="flex min-h-9 min-w-0 flex-1 items-center gap-3 rounded-md py-1.5 pl-1 text-left hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)]"
      >
        <AudioLines
          size={14}
          className={`shrink-0 ${active ? "text-[var(--accent)]" : "text-[var(--quiet)]"}`}
        />
        <div className="min-w-0 flex-1">
          <h4 className="truncate text-xs font-medium text-[var(--muted)]">{model.name}</h4>
          <p className="truncate text-[11px] text-[var(--quiet)]">
            {model.family.replace(/_/g, " ")} · {formatBytes(model.size)}
          </p>
        </div>
        {active && <Check size={14} className="shrink-0 text-[var(--accent)]" />}
      </button>
      {/* Only a family with a spec has anything to edit: a head is one phrase
          and its file is the whole record. */}
      {model.editable && (
        <button
          type="button"
          onClick={() => onEdit(model.path)}
          aria-label={`${model.name} settings`}
          title="Wake word settings"
          className="grid size-8 shrink-0 place-items-center rounded-md text-[var(--quiet)] hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
        >
          <Pencil size={14} />
        </button>
      )}
      {/* Deleting is the same hold-to-confirm as a recognition model: a wake
          word is one file and a stray click would take it away. */}
      <HoldToDelete
        onConfirm={() => void onRemove(model.path)}
        label={`Delete ${model.name}`}
        title="Delete model"
        className="grid size-8 shrink-0 place-items-center rounded-md text-[var(--quiet)] hover:bg-[var(--raised)] hover:text-[var(--danger)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
      >
        <Trash2 size={14} />
      </HoldToDelete>
    </div>
  );
}

function count(models: number): string {
  return models === 1 ? "1 model" : `${models} models`;
}
