import { createPortal } from "react-dom";
import { AlertTriangle, Check, Cpu, LoaderCircle, X } from "lucide-react";
import { useNotificationHost } from "./notificationHost";
import type { Notification, NotificationTone } from "./types";

type NotificationStackProps = {
  notifications: Notification[];
  /** The row whose text is on the clipboard right now. */
  copiedId: number | null;
  onDismiss: (id: number) => void;
  onCopy: (notification: Notification) => void;
};

/** The icon for a tone, sized for the badge that holds it. */
function ToneIcon({ tone, size }: { tone: NotificationTone; size: number }) {
  if (tone === "loading") return <LoaderCircle size={size} className="animate-spin" />;
  if (tone === "success") return <Check size={size} />;
  if (tone === "error") return <AlertTriangle size={size} />;
  return <Cpu size={size} />;
}

/** The badge's paint, which is the only part of a row that carries the tone. */
function badgeClass(tone: NotificationTone): string {
  if (tone === "error") return "bg-[color-mix(in_srgb,var(--danger)_13%,transparent)] text-[var(--danger)]";
  if (tone === "success") return "bg-[color-mix(in_srgb,var(--accent)_13%,transparent)] text-[var(--accent)]";
  return "bg-[color-mix(in_srgb,var(--accent)_13%,transparent)] text-[var(--accent)]";
}

/**
 * The notification stack: a question in the middle, reports in the corner.
 *
 * Presentational on purpose: it holds no state of its own and takes no action, so
 * every view that reports something renders the same rows rather than each
 * inventing its own idea of what an error looks like.
 *
 * **Two places, for two different jobs.** A report about something that already
 * happened is a corner toast -- it can be read late and expire without costing
 * anything. A question that is holding a send is centred: the user is looking at
 * the middle of the window, and a prompt they might not notice is a prompt that
 * parks their message.
 *
 * **Clicking a corner row copies its text.** A provider or engine failure arrives
 * as a long sentence, and the first thing anyone does with it is go and read it
 * somewhere else. The icon changes to a check for a moment to confirm.
 */
export default function NotificationStack({ notifications, copiedId, onDismiss, onCopy }: NotificationStackProps) {
  const host = useNotificationHost();

  if (notifications.length === 0) return null;

  const questions = notifications.filter((notification) => notification.anchor === "top");
  const reports = notifications.filter((notification) => notification.anchor !== "top");

  const close = (notification: Notification) => {
    // The caller raised the question, so it is the caller that knows what to put
    // back or cancel. The stack only removes the row.
    notification.onDismiss?.();
    onDismiss(notification.id);
  };

  // Portalled into the open dialog when there is one, because that is the only
  // place this can be drawn above a `showModal()` top layer. Into the body
  // otherwise, which is where it lived before the settings modal could raise one.
  const stack = (
    <>
      {questions.length > 0 && (
        <div aria-live="assertive" className="pointer-events-none fixed left-1/2 top-3 z-[130] flex w-[min(460px,calc(100vw-1.5rem))] -translate-x-1/2 flex-col gap-1.5">
          {questions.map((notification) => (
            <section
              key={notification.id}
              role={notification.tone === "confirm" ? "alertdialog" : "status"}
              aria-label={notification.title ?? "Question"}
              className="pz-question-in pointer-events-auto flex items-center gap-2.5 rounded-xl border border-[color-mix(in_srgb,var(--accent)_20%,var(--line))] bg-[color-mix(in_srgb,var(--panel)_95%,transparent)] py-1.5 pl-2 pr-1.5 shadow-[0_14px_40px_-22px_rgba(0,0,0,0.65)] backdrop-blur-lg"
            >
              <span className={`grid size-7 shrink-0 place-items-center rounded-full ${badgeClass(notification.tone)}`}>
                <ToneIcon tone={notification.tone} size={14} />
              </span>
              <div className="min-w-0 flex-1">
                {notification.title && <p className="truncate text-[12px] font-semibold leading-4 text-[var(--text)]">{notification.title}</p>}
                <p className="line-clamp-2 text-[11.5px] leading-4 text-[var(--muted)]">{notification.message}</p>
              </div>
              <div className="flex shrink-0 items-center gap-1">
                {notification.actions?.map((action) => (
                  <button
                    key={action.label}
                    type="button"
                    onClick={action.onSelect}
                    className={`min-h-7 rounded-md px-2 text-[11.5px] font-medium transition-colors focus-visible:outline-2 focus-visible:outline-[var(--accent)] ${
                      action.primary
                        ? "bg-[var(--accent)] text-[var(--accent-ink)] hover:opacity-90"
                        : "text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]"
                    }`}
                  >
                    {action.label}
                  </button>
                ))}
                <button
                  type="button"
                  onClick={() => close(notification)}
                  className="grid size-7 place-items-center rounded-md text-[var(--quiet)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
                  aria-label="Close notification"
                >
                  <X size={14} />
                </button>
              </div>
            </section>
          ))}
        </div>
      )}

      {reports.length > 0 && (
        <div aria-live="polite" className="pointer-events-none fixed right-4 top-4 z-[120] flex w-[min(400px,calc(100vw-2rem))] flex-col gap-2">
          {reports.map((notification) => {
            const copied = copiedId === notification.id;
            return (
              <div
                key={notification.id}
                className={`pz-toast-in pointer-events-auto flex items-start gap-2.5 overflow-hidden rounded-xl border bg-[color-mix(in_srgb,var(--panel)_93%,transparent)] py-2.5 pl-2.5 pr-2 shadow-[0_18px_44px_-24px_rgba(0,0,0,0.65)] backdrop-blur-md ${notification.tone === "error" ? "border-[color-mix(in_srgb,var(--danger)_32%,var(--line))]" : "border-[var(--line)]"}`}
              >
                <span className={`mt-0.5 grid size-7 shrink-0 place-items-center rounded-lg ${badgeClass(notification.tone)}`}>
                  {copied ? <Check size={15} /> : <ToneIcon tone={notification.tone} size={15} />}
                </span>
                <button
                  type="button"
                  onClick={() => onCopy(notification)}
                  className="min-w-0 flex-1 self-center text-left text-[12.5px] leading-5 text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
                  title="Click to copy notification"
                >
                  {notification.title && <span className="block text-[12px] font-semibold">{notification.title}</span>}
                  <span className="block break-words">{copied ? "Copied to clipboard" : notification.message}</span>
                </button>
                <button
                  type="button"
                  onClick={() => close(notification)}
                  className="grid size-7 shrink-0 place-items-center rounded-lg text-[var(--quiet)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
                  aria-label="Close notification"
                >
                  <X size={14} />
                </button>
              </div>
            );
          })}
        </div>
      )}
    </>
  );

  // `host` is always mounted -- it is the dialog, which exists whether or not it
  // is open -- so the null check is only about whether it is the right target.
  return createPortal(stack, host ?? document.body);
}
