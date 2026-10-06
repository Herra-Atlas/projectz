import { useEffect, useRef, useState } from "react";
import { Check, ChevronDown } from "lucide-react";

import {
  ACCESS_SHORT,
  PERMISSION_HINTS,
  PERMISSION_LABELS,
  type ChatMode,
  type PermissionMode,
} from "../features/chat/chatMode";

type ModeSelectorProps = {
  mode: ChatMode;
  permission: PermissionMode;
  /** False when the model reported that it cannot take a `tools` array. */
  canUseTools: boolean;
  onModeChange: (mode: ChatMode) => void;
  onPermissionChange: (permission: PermissionMode) => void;
};

/**
 * Picks Chat or Agent, and how much Agent may do.
 *
 * Sits on the left of the composer row, mirroring the model picker on the right:
 * the two ends of the row are what a user is choosing between, and the controls
 * between them are per-reply settings.
 *
 * The permission selector only appears in Agent mode. Offering it in Chat would
 * invite setting a level that has no effect, and a control whose effect is
 * invisible is worse than no control.
 */
export default function ModeSelector({ mode, permission, canUseTools, onModeChange, onPermissionChange }: ModeSelectorProps) {
  const [modeOpen, setModeOpen] = useState(false);
  const [permissionOpen, setPermissionOpen] = useState(false);
  const modeRef = useRef<HTMLDivElement>(null);
  const permissionRef = useRef<HTMLDivElement>(null);

  // Both menus close on an outside click, and only one at a time. Without the
  // second half the two overlays can stack on top of each other.
  useDismiss(modeRef, modeOpen, () => setModeOpen(false));
  useDismiss(permissionRef, permissionOpen, () => setPermissionOpen(false));

  return (
    <div className="flex min-h-8 items-center gap-2 px-1">
      <div ref={modeRef} className="relative">
        <button
          type="button"
          aria-expanded={modeOpen}
          aria-label={`Mode ${mode === "agent" ? "Agent" : "Chat"}`}
          onClick={() => { setModeOpen((open) => !open); setPermissionOpen(false); }}
          className="inline-flex h-6 items-center gap-1 rounded px-1.5 text-[10px] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
        >
          {/* The label recedes and the value does not, because the current state is
              what the user came to read. */}
          <span className="text-[var(--quiet)]">Mode</span>
          <span className={mode === "agent" ? "text-[var(--accent)]" : "text-[var(--text)]"}>
            {mode === "agent" ? "Agent" : "Chat"}
          </span>
          <ChevronDown size={11} className="text-[var(--quiet)]" />
        </button>

        {modeOpen && (
          <div className="absolute bottom-full left-0 z-40 mb-2 w-64 overflow-hidden rounded-md border border-[var(--line)] bg-[var(--panel)] p-1 shadow-xl">
            <p className="px-2 py-1.5 text-[10px] text-[var(--quiet)]">Mode</p>
            <ModeOption
              selected={mode === "chat"}
              title="Chat"
              hint="Answers with no tools. Use for questions and writing."
              onSelect={() => { onModeChange("chat"); setModeOpen(false); }}
            />
            <ModeOption
              selected={mode === "agent"}
              title="Agent"
              hint="Can read, search, edit files and run commands."
              disabled={!canUseTools}
              disabledHint={
                canUseTools
                  ? undefined
                  : "This model reported that it cannot call tools. Test the provider again to refresh this."
              }
              onSelect={() => { onModeChange("agent"); setModeOpen(false); }}
            />
          </div>
        )}
      </div>

      {mode === "agent" && (
        <div ref={permissionRef} className="relative">
          <button
            type="button"
            aria-expanded={permissionOpen}
            aria-label={`Permissions ${PERMISSION_LABELS[permission]}`}
            onClick={() => { setPermissionOpen((open) => !open); setModeOpen(false); }}
            className="inline-flex h-6 items-center gap-1 rounded px-1.5 text-[10px] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
          >
            <span className="text-[var(--quiet)]">Access</span>
            <span className={permission === "full" ? "text-[var(--danger)]" : "text-[var(--accent)]"}>
              {ACCESS_SHORT[permission]}
            </span>
            <ChevronDown size={11} className="text-[var(--quiet)]" />
          </button>

          {permissionOpen && (
            <div className="absolute bottom-full left-0 z-40 mb-2 w-64 overflow-hidden rounded-md border border-[var(--line)] bg-[var(--panel)] p-1 shadow-xl">
              <p className="px-2 py-1.5 text-[10px] text-[var(--quiet)]">Access</p>
              {(Object.keys(PERMISSION_LABELS) as PermissionMode[]).map((level) => (
                <ModeOption
                  key={level}
                  selected={permission === level}
                  title={PERMISSION_LABELS[level]}
                  hint={PERMISSION_HINTS[level]}
                  // Full is the only level that carries real risk of an unwanted
                  // change, so it is the one that reads in the danger colour.
                  danger={level === "full"}
                  onSelect={() => { onPermissionChange(level); setPermissionOpen(false); }}
                />
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function ModeOption({
  selected,
  title,
  hint,
  danger,
  disabled,
  disabledHint,
  onSelect,
}: {
  selected: boolean;
  title: string;
  hint: string;
  danger?: boolean;
  disabled?: boolean;
  disabledHint?: string;
  onSelect: () => void;
}) {
  const description = disabledHint ?? hint;
  return (
    <button
      type="button"
      disabled={disabled}
      aria-pressed={selected}
      onClick={onSelect}
      className={`flex w-full flex-col items-start gap-0.5 rounded px-2 py-1.5 text-left focus-visible:outline-2 focus-visible:outline-[var(--accent)] ${
        disabled
          ? "cursor-not-allowed opacity-50"
          : "hover:bg-[var(--raised)]"
      }`}
    >
      <span className="flex w-full items-center justify-between gap-2">
        <span className={`text-xs ${danger && !disabled ? "text-[var(--danger)]" : "text-[var(--text)]"}`}>
          {title}
        </span>
        {selected && <Check size={13} className="shrink-0 text-[var(--accent)]" />}
      </span>
      <span className="text-[10px] leading-4 text-[var(--quiet)]">{description}</span>
    </button>
  );
}

/** Closes `open` on an outside click or Escape.
 *
 * One shared hook because the two menus need identical behaviour and two copies
 * of this loop would be two places for them to drift.
 */
function useDismiss(
  ref: React.RefObject<HTMLDivElement | null>,
  open: boolean,
  close: () => void,
) {
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => {
      if (ref.current && !ref.current.contains(event.target as Node)) close();
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") close();
    };
    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open, close, ref]);
}