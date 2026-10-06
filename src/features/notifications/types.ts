/**
 * One message in the corner stack.
 *
 * `tone` is "loading" rather than a fourth colour: a loading row stays on screen
 * until it is replaced or cleared, while the other two expire on a timer. The
 * difference is in the lifetime, not the paint.
 */
export type NotificationTone = "loading" | "success" | "error";

export type Notification = {
  id: number;
  /** Groups repeated notifications about one subject. See `useNotifications`. */
  key?: string;
  tone: NotificationTone;
  message: string;
  /** Absolute timestamp, or null for a loading row that waits to be cleared. */
  expiresAt: number | null;
};

/**
 * The one function every view calls to report something.
 *
 * The optional `key` is what makes this usable for progress: a row that stays
 * put while its subject changes, rather than a new row every step. `ChatPage`
 * takes this exact shape for its `onNotify`, so it is narrowed to the two tones
 * that make sense for a one-off event.
 */
export type Notify = (tone: NotificationTone, message: string, key?: string) => void;
