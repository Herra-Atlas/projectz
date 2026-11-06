/**
 * The files on a clipboard event.
 *
 * A paste from Explorer or from a screenshot tool arrives as browser `File`
 * objects, which is the same shape a DOM drop hands over -- so a pasted file and
 * a dragged one take the same route into `useAttachments`.
 *
 * `files` is the spec's flattened list, but WebView2 has been seen to populate
 * `items` and leave `files` empty for some shell formats, so the items are read
 * as the fallback rather than pasting silently doing nothing.
 */
export function clipboardFiles(data: DataTransfer | null): File[] {
  if (!data) return [];
  const files = Array.from(data.files);
  if (files.length > 0) return files;
  return Array.from(data.items)
    .filter((item) => item.kind === "file")
    .map((item) => item.getAsFile())
    .filter((file): file is File => file !== null);
}
