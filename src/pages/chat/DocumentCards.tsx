import { ArrowUpRight, FileSpreadsheet, FileText, FileType } from "lucide-react";
import type { ActivityStep } from "../../features/chat/types";
import { collectDocuments, type ChatDocument } from "../../features/chat/documents";

type DocumentCardsProps = {
  steps: ActivityStep[];
  /**
   * Opens the file in the right panel.
   *
   * The same callback `FileChanges` takes, and for the same reason: the path is
   * workspace-relative already, which is the form the panel's read command wants,
   * and the file may sit in a folder nobody has expanded in the tree.
   */
  onOpenFile?: (path: string) => void;
};

/**
 * The documents a reply produced, below the answer.
 *
 * A `.docx` or a spreadsheet is the *result* of the reply rather than a step in
 * it, so it is drawn where the answer ends, not inside the activity panel.
 *
 * A row, not a card. The transcript already has one shape for "this reply
 * touched a file" -- the row above this component -- and a heavy panel made a
 * document look like it was competing with the answer. This is one compact row
 * with a hairline and a quiet fill: the file's name in the transcript's own
 * text, what kind it is underneath, and an arrow at the end that is always
 * there rather than a hover reward. Only the border, the icon and the arrow
 * move on hover -- the surface stays exactly as it is.
 */
export default function DocumentCards({ steps, onOpenFile }: DocumentCardsProps) {
  const documents = collectDocuments(steps);
  if (documents.length === 0) return null;

  return (
    <ul className="mt-1.5 flex w-full max-w-sm flex-col gap-1.5">
      {documents.map((document) => (
        <DocumentRow key={document.path} document={document} onOpenFile={onOpenFile} />
      ))}
    </ul>
  );
}

function DocumentRow({ document, onOpenFile }: { document: ChatDocument; onOpenFile?: (path: string) => void }) {
  return (
    <li className="min-w-0">
      <button
        type="button"
        onClick={() => onOpenFile?.(document.path)}
        disabled={!onOpenFile}
        title={onOpenFile ? `Open ${document.path}` : document.path}
        className="group flex w-full items-center gap-2.5 rounded-lg border border-[var(--line)] bg-[var(--panel)] py-1.5 pl-2 pr-2.5 text-left transition-colors hover:border-[var(--line-strong)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)] disabled:pointer-events-none"
      >
        <span className="shrink-0 text-[var(--quiet)] transition-colors group-hover:text-[var(--accent)]">
          <DocumentIcon document={document} />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate text-[12.5px] font-medium leading-4 text-[var(--text)]">{document.name}</span>
          <span className="block truncate text-[10.5px] leading-4 text-[var(--quiet)]">{document.label}</span>
        </span>
        {/* Always visible, at low contrast, rather than appearing on hover: a
            control nobody can see is one nobody knows is there. It brightens
            with the icon instead of the row changing colour. */}
        <ArrowUpRight
          size={13}
          aria-hidden
          className="shrink-0 text-[var(--quiet)] opacity-50 transition-opacity group-hover:opacity-100"
        />
      </button>
    </li>
  );
}

/** The glyph for the writer that produced the file. */
function DocumentIcon({ document }: { document: ChatDocument }) {
  switch (document.kind) {
    case "xlsx":
      return <FileSpreadsheet size={16} aria-hidden />;
    case "pdf":
      return <FileType size={16} aria-hidden />;
    case "docx":
      return <FileText size={16} aria-hidden />;
  }
}
