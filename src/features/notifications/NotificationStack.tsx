import { createPortal } from "react-dom";
import { Check, Clipboard, LoaderCircle, X } from "lucide-react";
import { useNotificationHost } from "./notificationHost";
import type { Notification } from "./types";

type NotificationStackProps = {
  notifications: Notification[];
  /** The row whose text is on the clipboard right now. */
  copiedId: number | null;
  onDismiss: (id: number) => void;
  onCopy: (notification: Notification) => void;
};

/**
 * The corner stack. One row per notification, newest at the bottom.
 *
 * Presentational on purpose: it holds no state of its own and takes no action, so
 * every view that reports something renders the same rows rather than each
 * inventing its own idea of what an error looks like.
 *
 * **Clicking a row copies its text.** A provider or engine failure arrives as a
 * long sentence, and the first thing anyone does with it is go and read it
 * somewhere else. The icon changes to a check for a moment to confirm.
 */
export default function NotificationStack({ notifications, copiedId, onDismiss, onCopy }: NotificationStackProps) {
  const host = useNotificationHost();

  if (notifications.length === 0) return null;

  // Portalled into the open dialog when there is one, because that is the only
  // place this can be drawn above a `showModal()` top layer. Into the body
  // otherwise, which is where it lived before the settings modal could raise one.
  const stack = (
    <div aria-live="polite" className="pointer-events-none fixed right-4 top-4 z-[120] flex w-[min(420px,calc(100vw-2rem))] flex-col gap-2">
      {notifications.map((notification) => {
        const copied = copiedId === notification.id;
        return (
          <div key={notification.id} className={`pointer-events-auto flex items-start gap-2 rounded-lg border bg-[var(--panel)] px-3 py-2.5 text-sm shadow-xl ${notification.tone === "error" ? "border-[color-mix(in_srgb,var(--danger)_40%,var(--line))] text-[var(--text)]" : "border-[var(--line)] text-[var(--text)]"}`}>
            <button type="button" onClick={() => onCopy(notification)} className="flex min-w-0 flex-1 items-center gap-2 text-left focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]" title="Click to copy notification">
              {copied
                ? <Check size={15} className="shrink-0 text-[var(--accent)]" />
                : notification.tone === "loading"
                  ? <LoaderCircle size={15} className="shrink-0 animate-spin text-[var(--accent)]" />
                  : notification.tone === "success"
                    ? <Check size={15} className="shrink-0 text-[var(--accent)]" />
                    : <Clipboard size={15} className="shrink-0 text-[var(--muted)]" />}
              <span className="min-w-0 break-words">{copied ? "Copied to clipboard" : notification.message}</span>
            </button>
            <button type="button" onClick={() => onDismiss(notification.id)} className="grid size-7 shrink-0 place-items-center rounded text-[var(--quiet)] hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]" aria-label="Close notification"><X size={15} /></button>
          </div>
        );
      })}
    </div>
  );

  // `host` is always mounted -- it is the dialog, which exists whether or not it
  // is open -- so the null check is only about whether it is the right target.
  return createPortal(stack, host ?? document.body);
}
