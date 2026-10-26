/**
 * The documents one reply produced, for the card below the answer.
 *
 * A `.docx`, `.xlsx` and `.pdf` have no line diff -- their bytes are a zip or a
 * drawing stream -- so `FileChanges` deliberately says nothing about them, and a
 * reply that produced a report would otherwise end with no evidence of it at all.
 * That is what this collects: the finished documents only.
 *
 * Text formats are left out on purpose. `write_document` reports a real diff for
 * `.md`, `.txt` and `.csv`, so `FileChanges` already names them with line counts
 * beside the answer; repeating one as a card would state the same fact twice and
 * make the card row look like duplication rather than a result.
 *
 * Only a successful call counts. A refused or failed write produced no file, and
 * a card is a claim that one exists.
 */

import type { ActivityStep } from "./types";
import { fileName } from "./fileChanges";

/** One finished document, as the transcript draws it. */
export type ChatDocument = {
  /** Workspace-relative, exactly as the tool reported it. */
  path: string;
  /** The part a reader scans for. */
  name: string;
  /** Which writer produced it, for the icon and the label. */
  kind: "docx" | "xlsx" | "pdf";
  /** Human wording for the kind, under the name. */
  label: string;
};

/**
 * The kinds worth a card, with the words each is called.
 *
 * An extension missing here is not drawn: a `.md` belongs to `FileChanges`, and
 * an unknown extension is not a document this app produced.
 */
const DOCUMENT_KINDS: Record<string, { kind: ChatDocument["kind"]; label: string }> = {
  docx: { kind: "docx", label: "Word document" },
  xlsx: { kind: "xlsx", label: "Spreadsheet" },
  pdf: { kind: "pdf", label: "PDF document" },
};

/** Every document this reply wrote, once each, in the order written. */
export function collectDocuments(steps: ActivityStep[]): ChatDocument[] {
  const documents = new Map<string, ChatDocument>();

  for (const step of steps) {
    if (step.kind !== "tool" || step.tool !== "write_document" || step.failed) continue;
    const path = step.detail;
    if (!path) continue;

    const extension = path.slice(path.lastIndexOf(".") + 1).toLowerCase();
    const entry = DOCUMENT_KINDS[extension];
    if (!entry) continue;

    // A path written twice is one document; a re-render replaces the file rather
    // than adding a second one, so the last call is the one that describes it.
    documents.set(path, { path, name: fileName(path), ...entry });
  }

  return [...documents.values()];
}
