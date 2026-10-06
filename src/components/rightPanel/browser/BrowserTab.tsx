import { useEffect, useRef } from "react";
import AddressBar from "./AddressBar";
import BrowserTabs from "./BrowserTabs";
import { useBrowserView } from "./useBrowserView";

/**
 * The Browser view: tabs over an address bar over a webview, and nothing else.
 *
 * **Mostly layout, because everything that can change belongs elsewhere.** The
 * webview's lifecycle and geometry are `useBrowserView`, the toolbar is
 * `AddressBar`, the strip is `BrowserTabs`, and this file is the box they share
 * plus the empty state the browser shows before it has been given anything to
 * open.
 *
 * **The webview is a native surface positioned over this component's box, not an
 * element inside it.** There is no `<iframe>` here and no DOM node for the page:
 * the backend creates a child webview in the window and the hook tells it where
 * to sit. That is what makes it a real browser -- and the reason the box is empty
 * on its own, which would otherwise look like a bug.
 */

type BrowserTabProps = {
  /**
   * Whether the browser may exist.
   *
   * True only while the panel is open *and* the browser is the view on screen.
   * Anything else is the webview being destroyed rather than hidden, because a
   * hidden one keeps its document, its scripts and its timers alive and would go
   * on costing memory for the rest of the session.
   */
  mounted: boolean;
  /**
   * A URL to open on arrival, set by the Files view's HTML preview.
   *
   * Consumed once per `requestKey`, so navigating away and back does not replay
   * a stale request over the page now showing.
   */
  initialUrl?: string | null;
  requestKey?: number | null;
  onRequestHandled?: () => void;
  onLastTabClosed?: () => void;
  /** Reported so the panel's owner can surface a refused address. */
  onNotify?: (tone: "success" | "error", message: string) => void;
};

export default function BrowserTab({ mounted, initialUrl, requestKey, onRequestHandled, onLastTabClosed, onNotify }: BrowserTabProps) {
  const browser = useBrowserView(mounted);
  const lastRequestRef = useRef<number | null>(null);

  // A file handed over from the Files view. Keyed so the same URL twice still
  // navigates, and guarded so it fires once per request rather than on every
  // re-render of the tab around it.
  useEffect(() => {
    if (!mounted || !initialUrl || requestKey == null || lastRequestRef.current === requestKey) return;
    lastRequestRef.current = requestKey;
    onRequestHandled?.();
    browser.navigate(initialUrl).catch((reason: unknown) => onNotify?.("error", String(reason)));
    // `browser` is deliberately not a dependency: it is a fresh object every
    // render, and depending on it would re-fire this on every keystroke in the
    // address bar. `navigate` is a stable callback.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mounted, initialUrl, requestKey]);

  // A tab with no page draws the empty state; tabs with pages share the one
  // surface. The strip only renders once tabs exist, so an untouched browser is
  // still just an address bar over the empty words.
  const showingEmpty = browser.tabs.length === 0 || (browser.tabs.length > 0 && !browser.url && !browser.loading);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <BrowserTabs
        tabs={browser.tabs}
        activeTab={browser.activeTab}
        onSelect={(id) => void browser.selectTab(id).catch((reason: unknown) => onNotify?.("error", String(reason)))}
        onClose={(id) => void browser.closeTab(id).then((lastClosed) => { if (lastClosed) onLastTabClosed?.(); }).catch((reason: unknown) => onNotify?.("error", String(reason)))}
        onNew={() => void browser.openTab().catch((reason: unknown) => onNotify?.("error", String(reason)))}
      />
      <AddressBar
        url={browser.url}
        canGoBack={browser.canGoBack}
        canGoForward={browser.canGoForward}
        loading={browser.loading}
        onNavigate={browser.navigate}
        onReload={browser.reload}
        onBack={browser.goBack}
        onForward={browser.goForward}
        onOpenExternally={browser.openExternally}
        onError={(message) => onNotify?.("error", message)}
      />

      {/*
        The webview's rectangle. Empty on purpose and never given a background:
        the native surface is painted *over* this box, so anything drawn here
        would either be hidden by the page or would sit on top of it.
      */}
      <div className="relative min-h-0 flex-1">
        <div ref={browser.containerRef} className="absolute inset-0" />
        {showingEmpty && <BrowserEmpty />}
      </div>
    </div>
  );
}

/**
 * What the browser shows before it has been given an address.
 *
 * **Shown until the first navigation, and the webview is not created until then.**
 * Creating it eagerly would cover this with a blank page that fetches nothing --
 * it would replace the words with an empty rectangle and cost a webview
 * allocation for a view nobody has asked to look at anything. Deferring the
 * creation means an opened-and-ignored browser tab costs no more than the address
 * bar it draws.
 */
function BrowserEmpty() {
  return (
    <div className="absolute inset-0 flex flex-col items-center justify-center px-6 text-center">
      <p className="text-sm text-[var(--text)]">Open a page</p>
      {/* The accent because it is the one line here that tells the reader what to
          do, and the app already means "this is an action" by that colour. */}
      <p className="mt-1 text-[13px] text-[var(--accent)]">Preview your app or any page.</p>
      <p className="mt-6 text-[13px] text-[var(--muted)]">Enter an address</p>
    </div>
  );
}
