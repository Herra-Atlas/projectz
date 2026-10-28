/**
 * Which renderable kind a file is, if any.
 *
 * `null` means source-only: no Preview button. A new format is one variant here
 * plus its renderer file and one line in `previewKindFor` -- nothing else moves.
 */
export type PreviewKind = "markdown" | "html" | "svg" | "image" | "pdf" | "docx" | "xlsx";

/**
 * Kinds with no source to edit, where the preview *is* the view.
 *
 * A `.docx` is a zip and a `.png` is pixels, so there is no text to put in the
 * editor. Two things follow, and both matter: the header offers no Source toggle
 * for these -- a toggle with one side empty is a lie -- and the editor's autosave
 * must not run, because it would read the file as UTF-8 and report a decode error
 * where a picture should be.
 *
 * `svg` is deliberately **not** one of these. It is a picture to a browser and
 * XML text to everyone else, so it keeps both views: the source is the default,
 * because a hand-written `.svg` is usually opened to be changed, and the Preview
 * toggle draws it.
 */
export function isRenderedOnly(kind: PreviewKind): boolean {
  return kind === "image" || kind === "pdf" || kind === "docx" || kind === "xlsx";
}

/**
 * Extensions the binary image preview draws.
 *
 * Only formats the webview itself can decode: a `.tif` is an image to a person
 * and an undrawable blob to a browser, and offering a Preview that can only ever
 * fail is worse than the source view it would replace. `jfif` is here because it
 * is JPEG under another name.
 */
const IMAGE_EXTENSIONS = ["png", "jpg", "jpeg", "jfif", "gif", "webp", "bmp", "ico", "avif"];

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
  if (extension === "md" || extension === "markdown" || extension === "mdown") return "markdown";
  if (extension === "html" || extension === "htm") return "html";
  // Before `image` for no reason but readability: `svg` is handled by the text
  // side of the panel, not the bytes side, so the two lists are disjoint anyway.
  if (extension === "svg") return "svg";
  if (IMAGE_EXTENSIONS.includes(extension)) return "image";
  if (extension === "pdf") return "pdf";
  if (extension === "docx") return "docx";
  if (extension === "xlsx") return "xlsx";
  return null;
}
