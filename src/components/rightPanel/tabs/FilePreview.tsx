import { useEffect, useRef, useState } from "react";
import { ChevronRight, Eye, FileText } from "lucide-react";
import type { TreeEntry } from "../../../features/rightPanel/useFileTree";
import { useFileEditor } from "../../../features/rightPanel/useFileEditor";
import { previewKindFor } from "./preview/previewKind";
import MarkdownPreview from "./preview/MarkdownPreview";

/**
 * One open file: back control, name, and its source or preview.
 *
 * Extracted from `FilesTab`, which held tree, search, request handling *and*
 * this. The header owns the Preview toggle: shown only when `previewKindFor`
 * names a kind, right-aligned past the filename so source stays the default.
 * The source is a plain editor with autosave through `useFileEditor`.
 */

type FilePreviewProps = {
  entry: TreeEntry;
  /** Absolute workspace path, so an HTML file can be handed to the browser. */
  workspace: string | null;
  onBack: () => void;
  /** Asked to show an HTML file in the Browser view rather than inline. */
  onPreviewInBrowser?: (fileUrl: string) => void;
  /** Reported so a failed save reaches the app's one notification stack. */
  onNotify?: (tone: "success" | "error", message: string) => void;
};

/**
 * Turns a workspace-relative path into a `file://` URL for the browser.
 *
 * Forward slashes throughout (the tree already speaks them on every platform),
 * encoded per segment so a space or `#` in a folder name survives the trip.
 * The backend re-checks containment, so this is addressing, not trust.
 */
export default function FilePreview({ entry, workspace, onBack, onPreviewInBrowser, onNotify }: FilePreviewProps) {
  const { text, state, error, set, retry, saveNow } = useFileEditor(entry.path);
  const notifyRef = useRef(onNotify);
  notifyRef.current = onNotify;
  const lastErrorRef = useRef<string | null>(null);

  // Source first, preview on request -- a toggle rather than a default, because
  // the tree is a code reader first and a renderer second.
  const [showingPreview, setShowingPreview] = useState(false);
  const kind = previewKindFor(entry.path);

  useEffect(() => {
    setShowingPreview(false);
    lastErrorRef.current = null;
  }, [entry.path]);

  useEffect(() => {
    if (state !== "error" || !error || lastErrorRef.current === error) return;
    lastErrorRef.current = error;
    notifyRef.current?.("error", error);
  }, [state, error]);

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
    if (text === undefined) {
      if (error) return <p className="px-3 py-2 text-xs text-[var(--danger)]">{error}</p>;
      return <p className="px-3 py-2 text-xs text-[var(--quiet)]">Reading…</p>;
    }
    if (showingPreview && kind === "markdown") return <MarkdownPreview text={text} />;
    return (
      <textarea
        value={text}
        onChange={(event) => set(event.target.value)}
        onBlur={saveNow}
        onKeyDown={(event) => {
          if ((event.ctrlKey || event.metaKey) && event.key === "s") {
            event.preventDefault();
            saveNow();
          }
          if (event.key === "Tab") {
            event.preventDefault();
            const target = event.currentTarget;
            const start = target.selectionStart ?? text.length;
            const end = target.selectionEnd ?? text.length;
            const next = `${text.slice(0, start)}  ${text.slice(end)}`;
            set(next);
            requestAnimationFrame(() => {
              target.selectionStart = target.selectionEnd = start + 2;
            });
          }
        }}
        spellCheck={false}
        aria-label={`Edit ${entry.path}`}
        className="min-h-0 flex-1 resize-none bg-transparent px-4 py-3 font-mono text-[12px] leading-5 text-[var(--text)] outline-none"
      />
    );
  };

  const saveLabel = state === "saving" ? "Saving…" : state === "dirty" ? "Unsaved" : state === "error" ? "Save failed" : "Saved";

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
        {state === "error" ? (
          <button
            type="button"
            onClick={retry}
            title={error ?? "Save failed"}
            className="shrink-0 rounded px-1.5 py-1 text-[11px] text-[var(--danger)] transition-colors hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
          >
            Retry
          </button>
        ) : (
          <span
            aria-live="polite"
            className={`shrink-0 text-[11px] ${state === "dirty" ? "text-[var(--accent)]" : "text-[var(--quiet)]"}`}
          >
            {saveLabel}
          </span>
        )}
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
