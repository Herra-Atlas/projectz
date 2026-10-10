/**
 * Which renderable kind a file is, if any.
 *
 * `null` means source-only: no Preview button. A new format is one variant here
 * plus its renderer file and one line in `previewKindFor` -- nothing else moves.
 */
export type PreviewKind = "markdown" | "html" | "image" | "pdf" | "docx" | "xlsx";

/**
 * Kinds with no source to edit, where the preview *is* the view.
 *
 * A `.docx` is a zip and a `.png` is pixels, so there is no text to put in the
 * editor. Two things follow, and both matter: the header offers no Source toggle
 * for these -- a toggle with one side empty is a lie -- and the editor's autosave
 * must not run, because it would read the file as UTF-8 and report a decode error
 * where a picture should be.
 */
export function isRenderedOnly(kind: PreviewKind): boolean {
  return kind === "image" || kind === "pdf" || kind === "docx" || kind === "xlsx";
}

/**
 * Names the preview a path gets, from its extension alone.
 *
 * Case-insensitive, because `README.MD` is as much Markdown as `readme.md`.
 * Kept extension-based rather than content-sniffed: it has to answer before the
 * file is read, so the header knows whether to offer the button at all.
 *
 * `svg` is deliberately absent. It is an image to a browser and text to everyone
 * else, and the source is the more useful of the two views in an editor's file
 * tree -- so it stays source-only rather than becoming a picture nobody can edit.
 */
export function previewKindFor(path: string): PreviewKind | null {
  const dot = path.lastIndexOf(".");
  const extension = dot < 0 ? "" : path.slice(dot + 1).toLowerCase();
  if (extension === "md" || extension === "markdown" || extension === "mdown") return "markdown";
  if (extension === "html" || extension === "htm") return "html";
  if (
    extension === "png" ||
    extension === "jpg" ||
    extension === "jpeg" ||
    extension === "gif" ||
    extension === "webp"
  ) {
    return "image";
  }
  if (extension === "pdf") return "pdf";
  if (extension === "docx") return "docx";
  if (extension === "xlsx") return "xlsx";
  return null;
}
