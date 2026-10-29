import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Keyboard, X } from "lucide-react";

import type { Notify } from "../../features/notifications/types";
import type { Keybinds } from "../../features/keybinds/types";
import { SettingsPageTitle, SettingRow, SettingsSection } from "../SettingsSection";

/**
 * The shortcuts the operating system owns.
 *
 * One row per shortcut, and one way to set it: press the field, press the keys.
 * Typing an accelerator by hand would be a second syntax for the thing the
 * recorder already produces, and the two would disagree on the first unusual
 * keyboard.
 *
 * Every save is checked by the backend before it is stored -- registering an
 * accelerator the OS refuses fails right here, at the field, rather than on the
 * next press when the user is trying to talk.
 */
export function KeybindsPage({ notify }: { notify: Notify }) {
  const [keybinds, setKeybinds] = useState<Keybinds | null>(null);
  const [capturing, setCapturing] = useState(false);

  const load = useCallback(() => {
    void invoke<Keybinds>("keybinds_get")
      .then(setKeybinds)
      .catch((reason: unknown) => notify("error", `Could not read the shortcuts: ${String(reason)}`));
  }, [notify]);

  useEffect(() => {
    load();
  }, [load]);

  const save = useCallback(
    (pushToTalk: string | null) => {
      void invoke<Keybinds>("keybinds_set", { pushToTalk })
        .then(setKeybinds)
        .catch((reason: unknown) => {
          notify("error", String(reason));
          load();
        });
    },
    [notify, load],
  );

  return (
    <>
      <SettingsPageTitle>
        <h2 className="text-[17px] font-semibold tracking-tight">Keybinds</h2>
      </SettingsPageTitle>

      <SettingsSection title="Voice">
        <SettingRow
          anchor="keybinds.pushToTalk"
          label="Push to talk"
          description="Press the shortcut to open the microphone from anywhere, even with another program in front. Press it again to stop, or just stop talking."
          hint="A recording still ends itself after a pause, so the shortcut only has to start it. With “Send when I stop talking” on, the words go out as a message; off, they land in the composer of whatever conversation is open — or a new one, which is created when you send."
          control={
            <ShortcutField
              value={keybinds?.pushToTalk ?? null}
              capturing={capturing}
              onCapture={setCapturing}
              onSet={(value) => save(value)}
              onClear={() => save(null)}
            />
          }
        />
      </SettingsSection>
    </>
  );
}

/**
 * A shortcut field: click, press the keys, done.
 *
 * Bound on `keydown` at the capture phase so the composer's own handlers never
 * see a combination being recorded -- pressing `Ctrl+F` to bind it would
 * otherwise open the app's find bar underneath the field.
 */
function ShortcutField({
  value,
  capturing,
  onCapture,
  onSet,
  onClear,
}: {
  value: string | null;
  capturing: boolean;
  onCapture: (capturing: boolean) => void;
  onSet: (accelerator: string) => void;
  onClear: () => void;
}) {
  const box = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!capturing) return;
    const onKey = (event: KeyboardEvent) => {
      event.preventDefault();
      event.stopPropagation();
      // A modifier alone is not a shortcut: the user is holding it on the way to
      // the key that completes the combination.
      if (["Control", "Shift", "Alt", "Meta"].includes(event.key)) return;
      if (event.key === "Escape") {
        onCapture(false);
        return;
      }
      const accelerator = acceleratorFor(event);
      if (!accelerator) return;
      onCapture(false);
      onSet(accelerator);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [capturing, onCapture, onSet]);

  useEffect(() => {
    if (capturing) box.current?.focus();
  }, [capturing]);

  return (
    <div className="flex items-center gap-1.5">
      <button
        ref={box}
        type="button"
        onClick={() => onCapture(!capturing)}
        aria-pressed={capturing}
        className={`inline-flex min-h-8 min-w-[150px] items-center justify-center gap-1.5 rounded-md border px-2.5 text-[12px] tabular-nums transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)] ${
          capturing
            ? "border-[var(--accent)] text-[var(--accent)]"
            : "border-[var(--line)] text-[var(--text)] hover:border-[var(--line-strong)] hover:bg-[var(--raised)]"
        }`}
      >
        <Keyboard size={13} />
        {capturing ? "Press the keys…" : (value ?? "Not set")}
      </button>
      {value && !capturing && (
        <button
          type="button"
          onClick={onClear}
          aria-label="Remove the shortcut"
          title="Remove the shortcut"
          className="grid size-8 shrink-0 place-items-center rounded-md text-[var(--quiet)] hover:bg-[var(--raised)] hover:text-[var(--danger)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
        >
          <X size={14} />
        </button>
      )}
    </div>
  );
}

/**
 * The accelerator a key press stands for, in the shape the OS registrar takes.
 *
 * `event.code` rather than `event.key`, because the key is what the layout says
 * (`é` on a French board) and the code is the physical key the OS registers.
 */
export function acceleratorFor(event: KeyboardEvent): string | null {
  const parts: string[] = [];
  if (event.ctrlKey) parts.push("Ctrl");
  if (event.metaKey) parts.push("Super");
  if (event.altKey) parts.push("Alt");
  if (event.shiftKey) parts.push("Shift");

  const code = event.code;
  let key: string | null = null;
  if (/^Key[A-Z]$/.test(code)) key = code.slice(3);
  else if (/^Digit[0-9]$/.test(code)) key = code.slice(5);
  else if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code)) key = code;
  else if (["Space", "Tab", "Enter", "Backspace", "Delete", "Insert", "Home", "End", "PageUp", "PageDown", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight"].includes(code)) {
    key = code;
  }

  // A bare letter or number would be a shortcut that eats ordinary typing, so a
  // combination always needs at least one modifier.
  if (!key || parts.length === 0) return null;
  return [...parts, key].join("+");
}
