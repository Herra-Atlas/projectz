import { createContext, useContext, useMemo, type ComponentProps, type ReactNode } from "react";
import type { Components, ExtraProps } from "react-markdown";
import CodeBlock from "./CodeBlock";

/** The app's one notification stack, narrowed to the tones a one-off event uses. */
type Notify = (tone: "success" | "error", message: string) => void;

/** What a rendered link or code block needs from the surface drawing it. */
type MarkdownActions = {
  openUrl: (url: string) => void;
  notify?: Notify;
};

/**
 * The actions, carried by context rather than closed over.
 *
 * See `markdownComponents` for why that distinction is load-bearing.
 */
const MarkdownActionsContext = createContext<MarkdownActions>({ openUrl: () => undefined });

/**
 * Supplies the actions every link and code block beneath it uses.
 *
 * Renders no element of its own, so it can wrap a layout without disturbing
 * it. In `ChatPage` it wraps the page; in the Sub agents panel it wraps the open
 * run -- the two surfaces that draw markdown.
 */
export function MarkdownActionsProvider({ openUrl, notify, children }: MarkdownActions & { children: ReactNode }) {
  const value = useMemo(() => ({ openUrl, notify }), [openUrl, notify]);
  return <MarkdownActionsContext.Provider value={value}>{children}</MarkdownActionsContext.Provider>;
}

/** A link, sent to the panel's browser rather than out to the system one. */
function MarkdownLink({ node, href, children, ...props }: ComponentProps<"a"> & ExtraProps) {
  const { openUrl } = useContext(MarkdownActionsContext);
  // `node` is react-markdown's own tree reference, and spreading it would land a
  // `node="[object Object]"` on the anchor. Destructured here only to drop it.
  void node;
  return (
    <a
      {...props}
      href={href}
      onClick={(event) => { event.preventDefault(); if (href) openUrl(href); }}
      onAuxClick={(event) => { event.preventDefault(); if (href) openUrl(href); }}
    >
      {children}
    </a>
  );
}

/** A fenced code block, drawn by `CodeBlock` with this surface's actions. */
function MarkdownPre(props: ComponentProps<"pre"> & ExtraProps) {
  const { openUrl, notify } = useContext(MarkdownActionsContext);
  return <CodeBlock {...props} onOpenUrl={openUrl} onNotify={notify} />;
}

/**
 * The element overrides both markdown surfaces render through.
 *
 * **One constant, every component in it defined at module level, on purpose.**
 * A component that is a fresh arrow built each render is a *different type* to
 * React, so React tears the old subtree down and mounts a new one. For a link
 * that is invisible; for a code block it is the whole bug -- the block is a scroll
 * container, so remounting builds a new element and `scrollTop` goes back to zero.
 * `App` re-renders on the notification sweep (every 250ms, because the sweep hands
 * back a fresh array), which was often enough that a code block snapped back to
 * the top before it could be read.
 *
 * A stable identity also keeps `Markdown`'s memo intact, so a reply that has not
 * changed is never parsed again.
 *
 * The chat draws a reply with these and the Sub agents panel draws a run's
 * transcript with them too, because it is the same data -- one definition, so a
 * link and a code block cannot come to behave differently in the two.
 */
export const markdownComponents: Components = {
  a: MarkdownLink,
  pre: MarkdownPre,
};
