import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

/** A file's bytes, ready to be drawn. */
export type FileBytes = {
  /**
   * The bytes themselves.
   *
   * Both forms are returned because both are needed: `<img>` and a canvas want a
   * `src`/`Blob`, and the document libraries want the `ArrayBuffer`. Handing back
   * one and re-reading for the other would mean reading a megabyte twice.
   */
  bytes: ArrayBuffer;
  /** An object URL for the same bytes, revoked when the file changes. */
  url: string;
};

/** What every binary preview is given: the path, and which root it is under. */
export type PreviewProps = {
  path: string;
  /** The installed skill whose folder `path` is relative to, or null. */
  skill?: string | null;
};

/**
 * The blob's declared type, from the file's extension.
 *
 * The bytes arrive with no name, and an image element is entitled to be told
 * what it was handed -- without this a `.bmp` or `.ico` is decoded only because
 * the browser chose to sniff it, and a `.pdf` blob is a bag of bytes with no
 * type at all. The list is the one `previewKindFor` draws, so a format cannot be
 * previewable and unlabelled at once.
 *
 * `application/octet-stream` is the honest answer for a `.docx`: it is a zip of
 * XML, and nothing reads it through the URL -- the Word preview takes the
 * `bytes` side.
 */
function mediaTypeFor(path: string): string {
  const dot = path.lastIndexOf(".");
  const extension = dot < 0 ? "" : path.slice(dot + 1).toLowerCase();
  switch (extension) {
    case "png":
      return "image/png";
    case "jpg":
    case "jpeg":
    case "jfif":
      return "image/jpeg";
    case "gif":
      return "image/gif";
    case "webp":
      return "image/webp";
    case "bmp":
      return "image/bmp";
    case "ico":
      return "image/x-icon";
    case "avif":
      return "image/avif";
    case "pdf":
      return "application/pdf";
    default:
      return "application/octet-stream";
  }
}

/**
 * Reads a file's bytes for the previews that cannot be text.
 *
 * One hook for the image, PDF and Word previews: all three ask the same command
 * for the same bytes and differ only in what they draw, so the object URL's
 * lifetime -- created on arrival, revoked on the way out -- is managed in one
 * place. A leaked object URL holds the whole file in memory for the rest of the
 * session, which for a scanned PDF is exactly the kind of thing that makes an app
 * feel heavy.
 */
export function useFileBytes(path: string, skill: string | null = null) {
  const [data, setData] = useState<FileBytes | undefined>(undefined);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    let created: string | null = null;
    setData(undefined);
    setError(null);

    invoke<ArrayBuffer>("panel_read_bytes", { relative: path, skill })
      .then((bytes) => {
        if (!active) return;
        created = URL.createObjectURL(new Blob([bytes], { type: mediaTypeFor(path) }));
        setData({ bytes, url: created });
      })
      .catch((reason: unknown) => {
        if (active) setError(String(reason));
      });

    return () => {
      active = false;
      // Revoked here rather than when the component unmounts, because the effect
      // re-runs on a path change and the old file is no longer on screen.
      if (created) URL.revokeObjectURL(created);
    };
  }, [path, skill]);

  return { data, error };
}
