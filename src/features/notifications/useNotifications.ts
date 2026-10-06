import { useCallback, useEffect, useRef, useState } from "react";
import type { Notification, Notify } from "./types";

/** How long a success or error row stays before it removes itself. */
const EXPIRY_MS = 6000;

/** How often expiry is re-checked. */
const SWEEP_MS = 250;

/**
 * The notification stack, as state.
 *
 * Owned here rather than in `App` so any view can be handed a `notify` without
 * the stack's shape being a detail of the app shell. `App` renders it once.
 *
 * **Two update rules, and both are load-bearing.**
 *
 * A `key` replaces the row carrying that key, which is how "Downloading X…"
 * becomes "X installed" without the earlier message being left behind or a second
 * row appearing. An unkeyed `loading` replaces every other unkeyed loading row,
 * because there is only ever one thing that can be loading at a time -- otherwise
 * a row no longer gets the spinner and just sits there looking finished.
 */
export function useNotifications() {
  const [notifications, setNotifications] = useState<Notification[]>([]);

  const notify = useCallback<Notify>((tone, message, key) => {
    const notification: Notification = {
      id: Date.now() + Math.random(),
      key,
      tone,
      message,
      expiresAt: tone === "loading" ? null : Date.now() + EXPIRY_MS,
    };
    setNotifications((current) => [
      ...current.filter((item) =>
        key ? item.key !== key : !(item.tone === "loading" && tone === "loading"),
      ),
      notification,
    ]);
  }, []);

  const dismiss = useCallback((id: number) => {
    setNotifications((current) => current.filter((notification) => notification.id !== id));
  }, []);

  /**
   * Removes a row by key.
   *
   * Separate from `dismiss` because a keyed row's id is not known to whoever
   * wants it gone -- the owner of a key is whoever reported the next state, and
   * that is a different place in the code.
   */
  const clearKey = useCallback((key: string) => {
    setNotifications((current) => current.filter((notification) => notification.key !== key));
  }, []);

  /** Drops every row matching a predicate, for callers that clear their own. */
  const clearWhere = useCallback((predicate: (notification: Notification) => boolean) => {
    setNotifications((current) => current.filter((notification) => !predicate(notification)));
  }, []);

  useEffect(() => {
    const timer = window.setInterval(() => {
      const now = Date.now();
      setNotifications((current) => current.filter((notification) => notification.expiresAt === null || notification.expiresAt > now));
    }, SWEEP_MS);
    return () => window.clearInterval(timer);
  }, []);

  return { notifications, notify, dismiss, clearKey, clearWhere };
}

export type NotificationStackControls = ReturnType<typeof useNotifications>;

/**
 * The copied-row id, which the stack owns because only it can expire it.
 *
 * Kept beside the stack rather than inside it: the row itself is presentational,
 * and a one-second timer that outlives a single click has no business in a
 * component that renders the text.
 */
export function useCopiedNotification() {
  const [copiedId, setCopiedId] = useState<number | null>(null);
  const timerRef = useRef<number | null>(null);

  const markCopied = useCallback((id: number) => {
    setCopiedId(id);
    if (timerRef.current !== null) window.clearTimeout(timerRef.current);
    timerRef.current = window.setTimeout(() => {
      setCopiedId((current) => (current === id ? null : current));
      timerRef.current = null;
    }, 1400);
  }, []);

  useEffect(() => () => {
    if (timerRef.current !== null) window.clearTimeout(timerRef.current);
  }, []);

  return { copiedId, markCopied };
}
