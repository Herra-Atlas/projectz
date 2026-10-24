/**
 * One message in the notification stack.
 *
 * `tone` is "loading" rather than a fourth colour: a loading row stays on screen
 * until it is replaced or cleared, while the other two expire on a timer. The
 * difference is in the lifetime, not the paint.
 *
 * `confirm` is a question, not a report: it carries `actions` and waits for one
 * of them, so it never expires on its own.
 */
export type NotificationTone = "loading" | "success" | "error" | "confirm";

/**
 * Where a row is drawn.
 *
 * The corner is for things that already happened and need reading once. `top` is
 * centred and reserved for a question that blocks a send: a prompt to answer
 * belongs in the middle of the screen, where the user is looking, not in a
 * corner they may not glance at.
 */
export type NotificationAnchor = "corner" | "top";

/** One button on a notification that asks something. */
export type NotificationAction = {
  label: string;
  onSelect: () => void;
  /** The action the user is most likely to want. At most one per row. */
  primary?: boolean;
};

export type Notification = {
  id: number;
  /** Groups repeated notifications about one subject. See `useNotifications`. */
  key?: string;
  tone: NotificationTone;
  /** A heading, for the rows large enough to carry one. */
  title?: string;
  message: string;
  /** Buttons that answer the row. Present only on a `confirm` notification. */
  actions?: NotificationAction[];
  /**
   * Runs when the row is closed with its X.
   *
   * A question that was dismissed is still a question the caller raised, and the
   * caller -- not the stack -- knows what to put back or cancel. Without this the
   * close button would silently strand the work the row was asking about.
   */
  onDismiss?: () => void;
  anchor: NotificationAnchor;
  /** Absolute timestamp, or null for a row that waits to be cleared. */
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
export type Notify = (tone: Exclude<NotificationTone, "confirm">, message: string, key?: string) => void;

/**
 * A notification that asks a question or stays while work is in progress.
 *
 * Its own function rather than a fourth argument on `Notify`: a prompt has to
 * carry actions, a title and a place on screen, and widening every call site to
 * pass three optional values for the one caller that needs them would make the
 * ordinary report harder to read.
 */
export type NotifyAction = (notification: {
  /** The row this replaces. Required when the row changes state, e.g. Loading. */
  key?: string;
  tone?: "confirm" | "loading";
  title?: string;
  message: string;
  actions?: NotificationAction[];
  anchor?: NotificationAnchor;
  onDismiss?: () => void;
}) => void;
