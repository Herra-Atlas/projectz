import { memo } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

type MarkdownProps = {
  /** The markdown source. */
  text: string;
  /**
   * Element overrides, already memoised by the caller. Identity is part of the
   * memo comparison, so a caller that rebuilds this object every render would
   * defeat the whole point of this component.
   */
  components?: React.ComponentProps<typeof ReactMarkdown>["components"];
};

/**
 * Rendered markdown, memoised on its own text.
 *
 * # Why this exists
 *
 * The reply that is streaming re-renders the chat once per token, and every one
 * of those renders walks the whole transcript. Without this component each pass
 * re-parsed *every* reply through `react-markdown` -- so a long conversation
 * paid to parse itself again on every token of the newest answer, and the cost
 * grew the further back the history went. That is what made the text arrive in
 * stutters rather than smoothly.
 *
 * Memoised on `text` and the caller's (stable) `components`, a reply that has not
 * changed is skipped outright: only the message still streaming is parsed, once
 * per frame instead of once per message.
 *
 * Not folded into a single renderer in one of the surfaces because a second one
 * -- the Sub agents panel -- draws the same data the same way, and a copy per
 * surface would be the same fix made twice.
 */
export const Markdown = memo(function Markdown({ text, components }: MarkdownProps) {
  return (
    <ReactMarkdown remarkPlugins={[remarkGfm]} components={components}>
      {text}
    </ReactMarkdown>
  );
});
