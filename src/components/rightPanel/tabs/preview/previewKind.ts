/**
 * Which renderable kind a file is, if any.
 *
 * `null` means source-only: no Preview button. A new format is one variant here
 * plus its renderer file and one line in `previewKindFor` -- nothing else moves.
 */
export type PreviewKind = "markdown" | "html";

/**
 * Names the preview a path gets, from its extension alone.
 *
 * Case-insensitive, because `README.MD` is as much Markdown as `readme.md`.
 * Kept extension-based rather than content-sniffed: it has to answer before the
 * file is read, so the header knows whether to offer the button at all.
 */
export function previewKindFor(path: string): PreviewKind | null {
  const dot = path.lastIndexOf(".");
  const extension = dot < 0 ? "" : path.slice(dot + 1).toLowerCase();
  // FUTURE: docx/pdf need a parser (mammoth.js renders docx to HTML client-side
  // with no backend change -- the text is already readable via panel_read_preview
  // for docx stored as text, but real .docx is a zip and needs the library).
  if (extension === "md" || extension === "markdown" || extension === "mdown") return "markdown";
  if (extension === "html" || extension === "htm") return "html";
  return null;
}
