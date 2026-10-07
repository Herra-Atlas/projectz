/**
 * Attachment handling for the composer.
 *
 * All three entry points -- the file picker, files dropped on the window by the
 * OS, and files dropped into the browser DOM -- converge on one `add`, so the
 * capability check and the de-duplication by path are stated once. Having a
 * second path that skipped either would let an image reach a model that cannot
 * read it, which fails silently at the provider rather than visibly here.
 *
 * The two drop routes are genuinely different and stay separate:
 *
 * - `addPaths` goes through Rust (`read_chat_attachment`), because a dropped
 *   path is on disk and reading it in the webview would need the filesystem
 *   permission this app deliberately does not hold in the frontend.
 * - `addBrowserFiles` has only a browser `File`, so it reads the bytes here and
 *   hands them over already base64'd.
 */

import { useCallback, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open as openFiles } from "@tauri-apps/plugin-dialog";
import { acceptsInput, type ModelCapabilities } from "../../features/models/modelCapabilities";
import type { AttachedFile } from "./types";

/** Matches the shared output budget: a 15 MB base64 payload is ~20 MB of prompt. */
const MAX_ATTACHMENT_BYTES = 15 * 1024 * 1024;

/**
 * What counts as attachable.
 *
 * Checked against the MIME type the browser reported *and* the extension,
 * because a dropped file often arrives with no type at all (`""`), and an
 * unknown-type `.ts` is a text file that should attach. The type list alone
 * rejects it; the extension alone would accept anything renamed `.txt`.
 */
const SUPPORTED_TYPE = /^(image\/(png|jpeg|webp|gif)|text\/.*|application\/(json|xml|pdf))$/;
const SUPPORTED_NAME = /\.(pdf|txt|md|markdown|log|json|csv|html?|xml|ya?ml|rs|py|js|jsx|ts|tsx|css|toml|sql|sh)$/i;

/**
 * Reads bytes without blowing the argument limit of `String.fromCharCode`.
 *
 * Spread a whole 15 MB array at once it overflows the call stack, so it is
 * chunked. 32 KB per spread is the conventional safe chunk.
 */
function toBase64(bytes: Uint8Array): string {
  let binary = "";
  for (let offset = 0; offset < bytes.length; offset += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + 0x8000));
  }
  return btoa(binary);
}

type UseAttachmentsOptions = {
  /** What the selected model reports it accepts. Null until it has been read. */
  capabilities: ModelCapabilities | null;
  onNotify: (tone: "success" | "error", message: string) => void;
};

export function useAttachments({ capabilities, onNotify }: UseAttachmentsOptions) {
  const [attachments, setAttachments] = useState<AttachedFile[]>([]);

  /**
   * Adds what the model can actually use.
   *
   * A model that reported it cannot read images should not have one dropped on
   * it: the request would be rejected or the image silently ignored. Documents
   * are inlined as text by the backend, so they are unaffected and stay allowed
   * everywhere -- the check is per file, not a blanket block on a mixed drop.
   *
   * De-duplicated by `path` rather than by name, so the same file dropped twice
   * is one chip while two files that happen to share a name stay two.
   */
  const add = useCallback((files: AttachedFile[]) => {
    const takesImages = acceptsInput(capabilities, "image");
    const documents = takesImages ? files : files.filter((file) => !file.mime_type.startsWith("image/"));
    if (!takesImages) {
      const images = files.length - documents.length;
      if (images > 0) {
        onNotify("error", images === 1 ? "This model cannot read images." : `${images} images skipped: this model cannot read images.`);
      }
    }
    setAttachments((current) => {
      const existing = new Set(current.map((file) => file.path));
      return [...current, ...documents.filter((file) => !existing.has(file.path) && Boolean(file.name))];
    });
  }, [capabilities, onNotify]);

  /** Files the OS dropped onto the window: real paths, so Rust reads them. */
  const addPaths = useCallback(async (paths: string[]) => {
    const results = await Promise.all(paths.map(async (path) => {
      try {
        return await invoke<AttachedFile>("read_chat_attachment", { path });
      } catch (reason) {
        onNotify("error", String(reason));
        return null;
      }
    }));
    add(results.filter((file): file is AttachedFile => file !== null));
  }, [add, onNotify]);

  /**
   * Files dropped into the page DOM, which arrive as browser `File` objects.
   *
   * Read here rather than in Rust because there is no path to hand over, and the
   * PDF case needs `extract_pdf_attachment` in the backend either way -- that one
   * takes bytes precisely because a browser drop has nothing else to give.
   */
  const addBrowserFiles = useCallback(async (files: File[]) => {
    const results = await Promise.all(files.map(async (file) => {
      if (file.size > MAX_ATTACHMENT_BYTES) {
        onNotify("error", `${file.name} is larger than 15 MB`);
        return null;
      }
      if (!SUPPORTED_TYPE.test(file.type) && !SUPPORTED_NAME.test(file.name)) {
        onNotify("error", `${file.name} is not a supported image, PDF, or text/code file`);
        return null;
      }
      try {
        const isPdf = file.type === "application/pdf" || file.name.toLowerCase().endsWith(".pdf");
        const dataBase64 = toBase64(new Uint8Array(await file.arrayBuffer()));
        if (isPdf) {
          return await invoke<AttachedFile>("extract_pdf_attachment", { name: file.name, dataBase64 });
        }
        // `lastModified` in the synthetic path is what makes a re-dropped file
        // replace its earlier chip instead of stacking a second copy.
        return {
          path: `browser:${file.name}:${file.lastModified}`,
          name: file.name,
          mime_type: file.type || "application/octet-stream",
          data_base64: dataBase64,
        };
      } catch (reason) {
        onNotify("error", `Could not read ${file.name}: ${String(reason)}`);
        return null;
      }
    }));
    add(results.filter((file): file is AttachedFile => file !== null));
  }, [add, onNotify]);

  const chooseFiles = useCallback(async () => {
    const selected = await openFiles({ multiple: true, directory: false });
    if (!selected) return;
    await addPaths(Array.isArray(selected) ? selected : [selected]);
  }, [addPaths]);

  /** Drops the sent files from the composer; they are now part of the transcript. */
  const removeSent = useCallback((sent: AttachedFile[]) => {
    setAttachments((current) => current.filter((file) => !sent.some((entry) => entry.path === file.path)));
  }, []);

  /**
   * Puts a failed send's files back.
   *
   * Beside the skill rollback in the send path: both describe a message that did
   * not go, and the user should not have to find the file again to retype it.
   */
  const restore = useCallback((files: AttachedFile[]) => {
    add(files);
  }, [add]);

  const remove = useCallback((path: string) => {
    setAttachments((current) => current.filter((file) => file.path !== path));
  }, []);

  return { attachments, add, addPaths, addBrowserFiles, chooseFiles, remove, removeSent, restore };
}