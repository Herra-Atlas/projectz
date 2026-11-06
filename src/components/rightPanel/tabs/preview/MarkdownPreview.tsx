import { useCallback } from "react";

import { Markdown } from "../../../../pages/chat/Markdown";
import {
  markdownComponents,
  MarkdownActionsProvider,
} from "../../../../pages/chat/markdownComponents";
import { isExternalLink, resolveLinkPath } from "../../../../features/rightPanel/fileLinks";
import { CONTENT_CLASS, CONTENT_STYLES } from "./contentStyles";

/**
 * Markdown rendered exactly like a chat reply.
 *
 * The same `react-markdown` + `remark-gfm` pair `ChatPage` uses and the same
 * element overrides, so a link and a code block read the same in a preview as in
 * a reply. The wrapper styles come from `contentStyles.ts`, which the Word
 * preview dresses its own converted HTML in -- one definition, shared.
 *
 * # Links go somewhere
 *
 * A previewed `.md` is usually a set of files that link to each other, so a link
 * is resolved against `basePath` and opened *in this panel* when it names a file
 * in the tree. Everything else -- an `https:` address, an anchor, a path that
 * climbs out of the open root -- goes to the caller's URL sink, which is the
 * panel's browser. Before this, the default anchor was left in place: the
 * webview navigated the whole panel away or nothing happened, and a link between
 * two notes was a dead word.
 */
export default function MarkdownPreview({
  text,
  basePath,
  onOpenPath,
  onOpenUrl,
}: {
  text: string;
  /** The previewed file's own root-relative path, which a link is written against. */
  basePath?: string;
  /** Opens a root-relative path in the Files view. */
  onOpenPath?: (path: string) => void;
  /** Everything that is not a file in this tree: the panel's browser. */
  onOpenUrl?: (url: string) => void;
}) {
  const openUrl = useCallback(
    (href: string) => {
      const target = basePath ? resolveLinkPath(basePath, href) : null;
      if (target) onOpenPath?.(target);
      // An address goes to the browser; anything else that did not resolve is a
      // path this tree cannot open (an anchor, or one that climbs above the
      // root), and a web search for a file name is worse than a link that does
      // nothing.
      else if (isExternalLink(href)) onOpenUrl?.(href);
    },
    [basePath, onOpenPath, onOpenUrl],
  );

  return (
    // A searchable scope: the preview is prose, which is what find is for, and it
    // scrolls itself, so a match is revealed inside this box rather than the panel.
    <div className="min-h-0 flex-1 overflow-auto px-4 py-3" data-find-scope="" data-find-label="file preview">
      <style>{CONTENT_STYLES}</style>
      <div className={`${CONTENT_CLASS} text-[13px] leading-7 text-[var(--text)]`}>
        <MarkdownActionsProvider openUrl={openUrl}>
          <Markdown text={text} components={markdownComponents} />
        </MarkdownActionsProvider>
      </div>
    </div>
  );
}
