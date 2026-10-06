import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/**
 * The panel browser's state and its lifecycle.
 *
 * **The webview is created and destroyed, never suspended.** A hidden webview
 * keeps its document, its JavaScript heap, its render tree and the page's own
 * timers alive, so a "closed" browser would go on costing memory and battery for
 * the rest of the session. So the one flag that decides everything is `mounted`:
 * false means the webview does not exist, and reopening starts a fresh load.
 *
 * Nothing is restored on reopen. Re-navigating to the page you just left is
 * itself a load nobody asked for, and restoring it would also mean holding the
 * URL somewhere the moment the user closed the tab.
 *
 * **Geometry is measured with a `ResizeObserver`, never a poll and never React
 * state.** A native webview is not part of the DOM, so it has to be told where it
 * is in the parent window's logical pixels. An observer fires only when the box
 * actually changes, which during the panel's drag is every frame -- and that drag
 * writes its width straight to an inline style and commits to React state only on
 * release, so a webview watching state would lag the entire gesture.
 */

/** One payload of the page-load event the backend emits. */
export type BrowserState = {
  url: string;
  canGoBack: boolean;
  canGoForward: boolean;
  loading: boolean;
  tabs: { id: number; url: string }[];
  activeTab: number | null;
};

/** The event the backend emits when a page finishes loading. */
const PAGE_EVENT = "panel-browser-page";

export type BrowserControls = {
  /** The page currently loaded, or empty before the first navigation. */
  url: string;
  canGoBack: boolean;
  canGoForward: boolean;
  loading: boolean;
  tabs: { id: number; url: string }[];
  activeTab: number | null;
  /**
   * False until the first navigation, which is when the webview is created.
   *
   * The empty state is drawn while this is false, so an opened-and-ignored
   * browser costs an address bar and nothing else. Creating the webview eagerly
   * would cover those words with a blank page that fetches nothing.
   */
  started: boolean;
  /** The element the webview is positioned over, for the observer to watch. */
  containerRef: React.RefObject<HTMLDivElement | null>;
  navigate: (input: string) => Promise<void>;
  openTab: (url?: string) => Promise<void>;
  selectTab: (id: number) => Promise<void>;
  closeTab: (id: number) => Promise<boolean>;
  reload: () => void;
  goBack: () => void;
  goForward: () => void;
  /** Opens the current page in the system browser. */
  openExternally: () => Promise<void>;
};

const EMPTY: BrowserState = { url: "", canGoBack: false, canGoForward: false, loading: false, tabs: [], activeTab: null };

export function useBrowserView(mounted: boolean): BrowserControls {
  const containerRef = useRef<HTMLDivElement>(null);
  const [state, setState] = useState<BrowserState>(EMPTY);
  /**
   * The URL currently *typed*, which is not the URL loaded.
   *
   * Kept apart from `state.url` so a failed navigation leaves the text the user
   * typed in the bar rather than replacing it with whatever page happened to load
   * -- losing what you typed because it did not work is the one thing an address
   * bar must never do.
   */
  const [draft, setDraft] = useState("");
  /** Whether the webview has been created yet. See `started`. */
  const [started, setStarted] = useState(false);

  /**
   * Whether the webview should exist.
   *
   * `mounted` alone is not enough: the webview is only created once there is
   * something to show it, so this is the panel being visible *and* a navigation
   * having been asked for. Destroying on `false` is what frees the page.
   */
  const alive = mounted && started;

  /**
   * Creates the webview, once per mount.
   *
   * Keyed on `alive` rather than `mounted` so the browser is not built until
   * there is an address to open it on, and torn down the moment the panel closes
   * or another view is brought forward.
   */
  useEffect(() => {
    if (!alive) return;
    // **No page is opened here.** The webview is created by `navigate`, because a
    // browser with nothing to show should not have a browsing surface at all.
    // Creating one and pointing it at a blank page would put `about:blank` into
    // the history, which is what makes a browser's back button go to an empty
    // page the first time it is pressed -- the most noticeable way for a browser
    // to feel broken.
    const unlistenPage = listen<BrowserState>(PAGE_EVENT, (event) => {
      setState(event.payload);
      setDraft("");
    });
    return () => {
      void unlistenPage.then((stop) => stop());
      // The real teardown, and the reason a closed browser costs nothing. A
      // hidden webview keeps its document, its scripts and its timers alive, so
      // it is destroyed outright rather than parked out of sight.
      void invoke("panel_browser_close").catch(() => undefined);
    };
  }, [alive]);

  /**
   * Follows the container and keeps the webview over it.
   *
   * The observer rather than a resize listener on the window, because the box
   * changes for three separate reasons -- a panel drag, a tab switch, and the
   * window itself moving -- and only the first two are about this element.
   *
   * Reported immediately on connect as well as on change: the first observation
   * is what actually puts the webview in the right place, because it was created
   * at 1x1 off-screen.
   */
  useEffect(() => {
    const container = containerRef.current;
    if (!alive || !container) return;

    const report = () => {
      const box = container.getBoundingClientRect();
      if (box.width < 1 || box.height < 1) {
        // A collapsed panel measures zero. Reporting it would be ignored anyway,
        // and hiding here as well means the webview is not left painted over the
        // chat while the panel animates closed.
        void invoke("panel_browser_set_visible", { visible: false }).catch(() => undefined);
        return;
      }
      void invoke("panel_browser_set_visible", { visible: true }).catch(() => undefined);
      void invoke("panel_browser_set_bounds", {
        x: box.left,
        y: box.top,
        width: box.width,
        height: box.height,
      }).catch(() => undefined);
    };

    const observer = new ResizeObserver(report);
    observer.observe(container);
    report();
    // Window moves do not resize this element, so the observer alone would leave
    // the webview behind when the window is dragged to another monitor. Cheap: it
    // fires on move, not on a timer, and reports the same geometry either way.
    const onWindowMove = () => report();
    window.addEventListener("resize", onWindowMove);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", onWindowMove);
    };
  }, [alive]);

  const navigate = useCallback(async (input: string) => {
    const trimmed = input.trim();
    if (!trimmed) return;
    setDraft(trimmed);
    try {
      // `panel_browser_navigate` creates the webview when there is not one, so
      // this single call is both "open the browser" and "go there". Nothing has
      // to be open beforehand, which is what lets an ignored browser tab cost
      // nothing at all.
      const next = await invoke<BrowserState>("panel_browser_navigate", { url: trimmed });
      setState(next);
      // Set only once the address was accepted, so a refused one leaves the empty
      // state showing rather than an empty native surface over it.
      setStarted(true);
    } catch (reason) {
      // The bar keeps what was typed. An address bar that clears itself on a
      // failed navigation throws away the thing the user needs to fix.
      setDraft(trimmed);
      throw reason;
    }
  }, []);

  const openTab = useCallback(async (url?: string) => {
    const trimmed = url?.trim() || undefined;
    if (trimmed) setDraft(trimmed);
    const next = await invoke<BrowserState>("panel_browser_open_tab", { url: trimmed ?? null });
    setState(next);
    // An empty tab draws the empty state, which needs no surface; a tab with a
    // URL creates one. `started` gates the webview, so only the latter sets it.
    if (trimmed) setStarted(true);
  }, []);

  const selectTab = useCallback(async (id: number) => {
    const next = await invoke<BrowserState>("panel_browser_select_tab", { id });
    setState(next);
  }, []);

  const closeTab = useCallback(async (id: number) => {
    const next = await invoke<BrowserState>("panel_browser_close_tab", { id });
    setState(next);
    // Closing the last tab destroys the surface, so the next navigation must
    // create it again rather than moving a webview that no longer exists.
    const lastClosed = next.tabs.length === 0;
    if (lastClosed) setStarted(false);
    return lastClosed;
  }, []);

  const reload = useCallback(() => {
    void invoke<BrowserState>("panel_browser_reload").then((next) => setState(next)).catch(() => undefined);
  }, []);

  const goBack = useCallback(() => {
    void invoke<BrowserState>("panel_browser_back").then((next) => setState(next)).catch(() => undefined);
  }, []);

  const goForward = useCallback(() => {
    void invoke<BrowserState>("panel_browser_forward").then((next) => setState(next)).catch(() => undefined);
  }, []);

  /**
   * Opens the page in the user's real browser.
   *
   * The opener plugin rather than a `window.open`, because a page opened in the
   * system browser inherits that browser's cookies and extensions -- which is the
   * point, and also the reason it is a deliberate button rather than a default.
   */
  const openExternally = useCallback(async () => {
    const url = state.url.trim();
    if (!url) return;
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    await openUrl(url);
  }, [state.url]);

  return {
    // The typed text wins while there is any, so a failed navigation is visible
    // rather than silently reverted.
    url: draft || state.url,
    canGoBack: state.canGoBack,
    canGoForward: state.canGoForward,
    loading: state.loading,
    tabs: state.tabs,
    activeTab: state.activeTab,
    started,
    containerRef,
    navigate,
    openTab,
    selectTab,
    closeTab,
    reload,
    goBack,
    goForward,
    openExternally,
  };
}
