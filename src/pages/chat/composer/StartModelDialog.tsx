/**
 * The "Start llama.cpp?" prompt.
 *
 * Split out because it is a self-contained dialog: given `starting`, it renders
 * and reports Confirm or Cancel. Nothing about the paused send leaks into it, so
 * the modal cannot grow a dependency on the run it is interrupting.
 *
 * Raised above the composer rather than inside it. Inside, it read as another
 * attachment on a draft the user is still editing.
 */

import { useEffect, useRef } from "react";
import { LoaderCircle } from "lucide-react";

type StartModelDialogProps = {
  starting: boolean;
  onConfirm: () => void;
  onCancel: () => void;
};

export default function StartModelDialog({ starting, onConfirm, onCancel }: StartModelDialogProps) {
  const cancelRef = useRef<HTMLButtonElement>(null);

  // Focus lands on Cancel, not Confirm. A paused send is most often a misclick on
  // a local model, and the destructive-looking action should not be the default.
  useEffect(() => {
    cancelRef.current?.focus();
  }, []);

  return (
    <div className="fixed inset-0 z-[140] grid place-items-center bg-black/65 p-4">
      <section
        role="dialog"
        aria-modal="true"
        aria-labelledby="start-model-title"
        className="w-full max-w-md rounded-xl border border-[var(--line)] bg-[var(--panel)] p-5 shadow-2xl"
        onKeyDown={(event) => {
          // Escape must not dismiss while the engine is coming up: the send is
          // already in flight, and closing the prompt would leave the user with
          // no way back to the message they typed.
          if (event.key === "Escape" && !starting) onCancel();
        }}
      >
        <h2 id="start-model-title" className="text-lg font-semibold">Start llama.cpp?</h2>
        <p className="mt-2 text-sm text-[var(--muted)]">The selected local model needs to be loaded before you can chat. Do you want to start llama.cpp now?</p>
        <div className="mt-5 flex justify-end gap-2">
          <button ref={cancelRef} type="button" onClick={onCancel} disabled={starting} className="min-h-10 rounded-md border border-[var(--line)] px-4 text-sm text-[var(--muted)] hover:bg-[var(--raised)] disabled:opacity-50">No</button>
          <button type="button" onClick={onConfirm} disabled={starting} className="inline-flex min-h-10 items-center gap-2 rounded-md bg-[var(--accent)] px-4 text-sm font-semibold text-[var(--accent-ink)] hover:opacity-90 disabled:opacity-50">
            {starting && <LoaderCircle size={15} className="animate-spin" />}
            {starting ? "Starting…" : "Yes, start"}
          </button>
        </div>
      </section>
    </div>
  );
}