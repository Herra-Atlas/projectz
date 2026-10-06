import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ChevronRight, Eye, FileText } from "lucide-react";
import type { TreeEntry } from "../../../features/rightPanel/useFileTree";
import { previewKindFor } from "./preview/previewKind";
import MarkdownPreview from "./preview/MarkdownPreview";

/**
 * One open file: back control, name, and its source or preview.
 *
 * Extracted from `FilesTab`, which held tree, search, request handling *and*
 * this. The header owns the Preview toggle: shown only when `previewKindFor`
 * names a kind, right-aligned past the filename so source stays the default.
 */

type FilePreviewProps = {
  entry: TreeEntry;
  /** Absolute workspace path, so an HTML file can be handed to the browser. */
  workspace: string | null;
  onBack: () => void;
  /** Asked to show an HTML file in the Browser view rather than inline. */
  onPreviewInBrowser?: (fileUrl: string) => void;
};

/**
 * Reads a file for preview, reporting what went wrong rather than throwing.
 *
 * The command returns the file's text as a bare string, not an object -- so this
 * is typed as `string` rather than reaching for `.text` on the result. Reading a
 * property the command never set is silent: the optional chain yields `undefined`,
 * the `?? ""` turns that into an empty string, and the preview renders a blank
 * panel that looks exactly like an empty file.
 */
async function readPreview(path: string): Promise<string> {
  return invoke<string>("panel_read_preview", { relative: path });
}

export default function FilePreview({ entry, workspace, onBack, onPreviewInBrowser }: FilePreviewProps) {
  // `undefined` while loading, `null` on failure, and a string once read. Three
  // states rather than two because an *empty* file is a real answer -- keyed on
  // `null` alone, `""` reads as still-loading and the panel says "Reading…" over a
  // file that simply has nothing in it.
  const [text, setText] = useState<string | undefined>(undefined);
  const [error, setError] = useState<string | null>(null);
  // Source first, preview on request -- a toggle rather than a default, because
  // the tree is a code reader first and a renderer second.
  const [showingPreview, setShowingPreview] = useState(false);
  const kind = previewKindFor(entry.path);

  useEffect(() => {
    let active = true;
    setText(undefined);
    setError(null);
    setShowingPreview(false);
    readPreview(entry.path)
      .then((value) => { if (active) setText(value); })
      .catch((reason: unknown) => { if (active) setError(String(reason)); });
    return () => { active = false; };
  }, [entry.path]);

  /**
   * Turns a workspace-relative path into a `file://` URL for the browser.
   *
   * Forward slashes throughout (the tree already speaks them on every platform),
   * encoded per segment so a space or `#` in a folder name survives the trip.
   * The backend re-checks containment, so this is addressing, not trust.
   */
  const fileUrlFor = (relative: string): string | null => {
    if (!workspace) return null;
    const root = workspace.replace(/\\/g, "/").replace(/\/+$/, "");
    const encoded = relative.split("/").map((segment) => encodeURIComponent(segment)).join("/");
    return `file://${root}/${encoded}`;
  };

  const handlePreview = () => {
    // HTML is a page, not a document: it belongs in the real browser with its
    // own layout engine, not in an inline frame. Markdown stays inline.
    if (kind === "html") {
      const url = fileUrlFor(entry.path);
      if (url && onPreviewInBrowser) {
        onPreviewInBrowser(url);
        return;
      }
    }
    setShowingPreview((showing) => !showing);
  };

  const body = () => {
    if (error) return <p className="px-3 py-2 text-xs text-[var(--danger)]">{error}</p>;
    if (text === undefined) return <p className="px-3 py-2 text-xs text-[var(--quiet)]">Reading…</p>;
    if (showingPreview && kind === "markdown") return <MarkdownPreview text={text} />;
    // Numbers on the left, unpadded and in a monospace face, so the gutter
    // aligns and a row can still be counted as a line. The number column is
    // rendered per row rather than by a `<pre>` because a single pre cannot
    // align a number gutter with wrapped content.
    if (text.length === 0) return <p className="px-3 py-2 text-xs text-[var(--quiet)]">This file is empty.</p>;
    return (
      <div className="min-h-0 flex-1 overflow-auto">
        {text.split("\n").map((line, index) => (
          <div key={index} className="flex hover:bg-[var(--raised)]">
            <span className="sticky left-0 w-9 shrink-0 select-none bg-[var(--page)] pr-2 text-right text-[11px] leading-5 tabular-nums text-[var(--quiet)]">{index + 1}</span>
            <code className="whitespace-pre-wrap break-all pr-2 text-[12px] leading-5 text-[var(--text)]">{line}</code>
          </div>
        ))}
      </div>
    );
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {/* A back control rather than a tab: the tree and the file are two states
          of one view, and a tab strip would imply they could both be open at
          once, which they cannot. */}
      <div className="flex min-h-9 shrink-0 items-center gap-1 border-b border-[var(--line)] px-2">
        <button type="button" onClick={onBack} aria-label="Back to tree" title="Back to tree" className="grid size-6 shrink-0 place-items-center rounded text-[var(--muted)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]">
          <ChevronRight size={14} className="rotate-180" />
        </button>
        <FileText size={13} className="shrink-0 text-[var(--quiet)]" />
        <span className="min-w-0 flex-1 truncate text-xs text-[var(--muted)]">{entry.path}</span>
        {kind && (
          <button
            type="button"
            onClick={handlePreview}
            aria-pressed={showingPreview}
            title={kind === "html" ? "Open in browser" : showingPreview ? "Show source" : "Preview"}
            className={`inline-flex shrink-0 items-center gap-1 rounded px-1.5 py-1 text-[11px] transition-colors focus-visible:outline-2 focus-visible:outline-[var(--accent)] ${showingPreview ? "bg-[var(--raised)] text-[var(--text)]" : "text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]"}`}
          >
            <Eye size={13} />
            <span>{kind === "html" ? "Open in browser" : showingPreview ? "Source" : "Preview"}</span>
          </button>
        )}
      </div>

      {body()}
    </div>
  );
}
