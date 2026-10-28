import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import type { Notify } from "../../features/notifications/types";
import type { SttInputs, SttSettings } from "../../features/stt/types";
import { SettingRow } from "../SettingsSection";
import Select from "../Select";

/**
 * The microphone dictation records from.
 *
 * The list is what this machine reports **now**, not what was remembered: a
 * microphone unplugged since the choice must be visibly gone, because the
 * alternative is a name that looks chosen and fails on the first press. The
 * stored answer is a name rather than an index, so plugging something else in
 * cannot silently move the choice to a different device.
 *
 * `System default` leads the list and is what "nothing chosen" means -- a machine
 * that never picked one has always recorded from it, and the row says which
 * device that currently resolves to instead of leaving it a phrase.
 */
export function MicrophoneSection({ notify }: { notify: Notify }) {
  const [inputs, setInputs] = useState<SttInputs | null>(null);
  const [device, setDevice] = useState<string | null>(null);

  const load = useCallback(() => {
    void invoke<SttInputs>("stt_inputs")
      .then(setInputs)
      .catch((reason: unknown) =>
        notify("error", `Could not list the microphones: ${String(reason)}`),
      );
    void invoke<SttSettings>("stt_settings")
      .then((settings) => setDevice(settings.device))
      .catch(() => undefined);
  }, [notify]);

  useEffect(() => {
    load();
  }, [load]);

  const choose = useCallback(
    (value: string) => {
      const next = value === "" ? null : value;
      setDevice(next);
      void invoke<SttSettings>("stt_select_input", { device: next })
        .then((settings) => setDevice(settings.device))
        // The select already shows the new value, so the record is re-read rather
        // than the control left claiming a choice the backend refused.
        .catch((reason: unknown) => {
          notify("error", `That microphone could not be chosen: ${String(reason)}`);
          load();
        });
    },
    [load, notify],
  );

  // A chosen device that is no longer plugged in keeps its own entry: it is still
  // the answer to "what will it try to open", and replacing it with the system
  // default would claim a choice nobody made.
  const missing = device !== null && inputs !== null && !inputs.devices.includes(device);

  const detail =
    device !== null
      ? device
      : inputs?.default
        ? `System default · ${inputs.default}`
        : "System default.";

  return (
    <section className="mb-8">
      <h3 className="mb-1.5 px-1 text-[11px] font-medium text-[var(--quiet)]">Input</h3>
      <div className="divide-y divide-[var(--line)] border-y border-[var(--line)]">
        <SettingRow
          anchor="system.microphone"
          label="Microphone"
          description={detail}
          hint="The device dictation records from. The app opens it only while the microphone button is held, so nothing listens in the background."
          control={
            <Select
              label="Microphone"
              value={device ?? ""}
              onChange={choose}
              disabled={inputs === null}
              size="sm"
              options={[
                { value: "", label: "System default" },
                ...(missing && device ? [{ value: device, label: `${device} · not connected` }] : []),
                ...(inputs?.devices ?? []).map((name) => ({ value: name, label: name })),
              ]}
              className="w-full max-w-[18rem] shrink-0"
            />
          }
        />
      </div>
    </section>
  );
}
