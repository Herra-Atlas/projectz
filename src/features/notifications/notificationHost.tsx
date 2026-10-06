import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from "react";

/**
 * Where the notification stack should render.
 *
 * **This exists because a native `<dialog>` opened with `showModal()` is not in
 * the document's normal rendering order.** The browser promotes it to the *top
 * layer*, painted above every `z-index` on the page -- including the stack's
 * `z-[120]`. No value of `z-index` can lift a fixed element above it, because
 * the two are not in the same stacking order to begin with.
 *
 * The fix is to stop fighting the cascade and render *inside* the dialog instead.
 * A child of the dialog is in the top layer along with it, so it wins
 * automatically and keeps the same top-right position it has everywhere else.
 *
 * So the stack needs to know which dialog, if any, is open. That is this. It
 * holds the element, or `null` when nothing is open -- the ordinary case, which
 * renders into `document.body` exactly as it always did.
 */
type NotificationHostContextValue = {
  /** The dialog currently open, or null. */
  host: HTMLElement | null;
  /** Called by a dialog as it opens and closes. */
  setHost: (element: HTMLElement | null) => void;
};

const NotificationHostContext = createContext<NotificationHostContextValue | null>(null);

export function NotificationHostProvider({ children }: { children: ReactNode }) {
  const [host, setHost] = useState<HTMLElement | null>(null);
  // Memoised so the value keeps its identity across unrelated re-renders; a fresh
  // object each time would re-render every consumer on every keystroke anywhere.
  const value = useMemo(() => ({ host, setHost }), [host]);
  return <NotificationHostContext.Provider value={value}>{children}</NotificationHostContext.Provider>;
}

/**
 * Claims the host for as long as `open` is true and `element` exists.
 *
 * The caller passes the element it already keeps a ref to, rather than this
 * hook owning a ref of its own -- a dialog and its ref are one thing, and two
 * refs over the same node is one more thing to keep in step.
 *
 * Clearing on unmount as well as on close is what stops a closed dialog's
 * element from being left in the context, where the stack would portal into a
 * subtree nobody can see.
 */
export function useClaimNotificationHost(open: boolean, element: HTMLElement | null) {
  const context = useContext(NotificationHostContext);
  const claim = context?.setHost;
  const active = open && element !== null;

  useEffect(() => {
    if (!claim) return;
    claim(active ? element : null);
    return () => { claim(null); };
  }, [claim, active, element]);
}

export function useNotificationHost() {
  return useContext(NotificationHostContext)?.host ?? null;
}

/** The setter alone, for a caller that owns its own lifecycle. */
export function useSetNotificationHost() {
  const context = useContext(NotificationHostContext);
  return useCallback((element: HTMLElement | null) => context?.setHost(element), [context]);
}
