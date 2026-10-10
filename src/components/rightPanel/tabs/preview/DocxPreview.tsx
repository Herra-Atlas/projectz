import { useEffect, useState } from "react";
import { FileText } from "lucide-react";
import PreviewNote from "./PreviewNote";
import { CONTENT_CLASS, CONTENT_STYLES } from "./contentStyles";
import { useFileBytes } from "./useFileBytes";

/**
 * A Word document, rendered as it reads.
 *
 * A `.docx` cannot be drawn by the browser, so the document has to be *converted*
 * rather than displayed: `mammoth` maps the file's styles onto plain HTML --
 * headings to `h1`..`h6`, lists to `ul`/`ol`, tables to `table`, embedded pictures
 * to inline images -- which the shared document styles then dress. It is a
 * conversion and not a faithful paginated replica, which is the right trade for a
 * side panel: the text, the structure and the pictures are what a reader wants,
 * and Word's own layout engine is what Word is for.
 *
 * The library is imported when a Word file is actually opened, so its weight is
 * never paid by someone reading source code.
 */
export default function DocxPreview({ path }: { path: string }) {
  const { data, error } = useFileBytes(path);
  const [html, setHtml] = useState<string | null>(null);
  const [failure, setFailure] = useState<string | null>(null);

  useEffect(() => {
    if (!data) return;
    let active = true;
    setFailure(null);
    setHtml(null);

    void (async () => {
      try {
        const mammoth = await import("mammoth");
        const result = await mammoth.convertToHtml({ arrayBuffer: data.bytes });
        if (active) setHtml(result.value);
      } catch (reason) {
        if (active) setFailure(reason instanceof Error ? reason.message : String(reason));
      }
    })();

    return () => {
      active = false;
    };
  }, [data]);

  if (error) return <PreviewNote tone="error">{error}</PreviewNote>;
  if (failure) return <PreviewNote tone="error">Could not read this document: {failure}</PreviewNote>;
  if (html === null) return <PreviewNote>Reading…</PreviewNote>;

  if (!html.trim()) {
    return (
      <PreviewNote>
        This document has no text to show. It may contain only pictures, which this view does not draw.
      </PreviewNote>
    );
  }

  return (
    <div className="min-h-0 flex-1 overflow-auto px-4 py-3">
      <style>{CONTENT_STYLES}</style>
      <div className="mb-2 flex items-center gap-1.5 text-[11px] text-[var(--quiet)]">
        <FileText size={12} />
        <span>Converted from the document's own styles</span>
      </div>
      {/* The HTML is produced by mammoth from a file on disk, not from anything a
          model wrote, and it is markup rather than script. */}
      <div className={`${CONTENT_CLASS} text-[13px] leading-7 text-[var(--text)]`} dangerouslySetInnerHTML={{ __html: html }} />
    </div>
  );
}
