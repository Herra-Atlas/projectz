import { useCallback, useEffect, useRef, useState } from "react";

/** How long the pointer has to stay down. */
const HOLD_MS = 2000;

/**
 * A delete control that only fires after the pointer has stayed on it.
 *
 * Delete buttons in settings remove files and records that are not in the undo
 * history: a model takes minutes to download again, a provider key has to be
 * found again. A confirm dialog would do the job too, but it asks a second
 * question with the same finger -- holding is the same gesture continued, and the
 * fill under the icon is the answer to "did it register".
 *
 * Pointer events rather than click, because a click is exactly what a misclick
 * produces. Releasing or leaving cancels, and the fill drains, so the control
 * never looks armed after the fact.
 */
export function HoldToDelete({
  onConfirm,
  label,
  title,
  disabled = false,
  className = "",
  children,
}: {
  onConfirm: () => void;
  /** Accessible name. The visible content is usually an icon. */
  label: string;
  title?: string;
  disabled?: boolean;
  className?: string;
  children: React.ReactNode;
}) {
  const [progress, setProgress] = useState(0);
  const frame = useRef<number | null>(null);
  const started = useRef<number | null>(null);

  const cancel = useCallback(() => {
    if (frame.current !== null) {
      cancelAnimationFrame(frame.current);
      frame.current = null;
    }
    started.current = null;
    setProgress(0);
  }, []);

  useEffect(() => cancel, [cancel]);

  const begin = useCallback(() => {
    if (disabled || started.current !== null) return;
    started.current = performance.now();
    const step = () => {
      if (started.current === null) return;
      const ratio = Math.min(1, (performance.now() - started.current) / HOLD_MS);
      setProgress(ratio);
      if (ratio >= 1) {
        started.current = null;
        frame.current = null;
        setProgress(0);
        onConfirm();
        return;
      }
      frame.current = requestAnimationFrame(step);
    };
    frame.current = requestAnimationFrame(step);
  }, [disabled, onConfirm]);

  return (
    <button
      type="button"
      disabled={disabled}
      onPointerDown={begin}
      onPointerUp={cancel}
      onPointerLeave={cancel}
      onPointerCancel={cancel}
      onBlur={cancel}
      aria-label={label}
      title={title ?? `${label} — hold to confirm`}
      className={`relative overflow-hidden ${className}`}
    >
      {/* The fill says how much of the hold is done. It is behind the icon and
          takes no pointer events, so the button is still one target. */}
      <span
        aria-hidden="true"
        className="pointer-events-none absolute inset-y-0 left-0 bg-[color-mix(in_srgb,var(--danger)_32%,transparent)]"
        style={{ width: `${progress * 100}%` }}
      />
      <span className="pointer-events-none relative contents">{children}</span>
    </button>
  );
}
