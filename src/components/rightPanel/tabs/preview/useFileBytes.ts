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
export function useFileBytes(path: string) {
  const [data, setData] = useState<FileBytes | undefined>(undefined);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    let created: string | null = null;
    setData(undefined);
    setError(null);

    invoke<ArrayBuffer>("panel_read_bytes", { relative: path })
      .then((bytes) => {
        if (!active) return;
        created = URL.createObjectURL(new Blob([bytes]));
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
  }, [path]);

  return { data, error };
}
