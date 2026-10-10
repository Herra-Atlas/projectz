import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { CONTENT_CLASS, CONTENT_STYLES } from "./contentStyles";

/**
 * Markdown rendered exactly like a chat reply.
 *
 * The same `react-markdown` + `remark-gfm` pair `ChatPage` uses, and the same
 * styles the Word preview dresses its converted HTML in -- shared shape, one
 * definition, in `contentStyles.ts`. Kept beside the file preview rather than
 * imported from the chat page because that page owns a 1300-line run lifecycle
 * nobody here needs.
 */
export default function MarkdownPreview({ text }: { text: string }) {
  return (
    <div className="min-h-0 flex-1 overflow-auto px-4 py-3">
      <style>{CONTENT_STYLES}</style>
      <div className={`${CONTENT_CLASS} text-[13px] leading-7 text-[var(--text)]`}>
        <ReactMarkdown remarkPlugins={[remarkGfm]}>{text}</ReactMarkdown>
      </div>
    </div>
  );
}
