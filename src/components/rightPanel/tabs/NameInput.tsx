import { useEffect, useRef, useState } from "react";
import { FileText, Folder } from "lucide-react";

type NameInputProps = {
  /** The current name when renaming, or a suggestion when creating. */
  initialValue: string;
  /** Select just the stem, so typing a new name keeps the extension. */
  selectStem?: boolean;
  onCommit: (name: string) => void;
  onCancel: () => void;
};

/**
 * The inline field for naming a row -- a rename, or a file about to exist.
 *
 * Inline rather than a modal prompt, for the same reason `Sidebar` renames a
 * session in place: the name belongs to the row it will become, and a system
 * prompt cannot be styled. Enter commits, Escape cancels, and a blur commits --
 * clicking away from a half-typed name to glance at something else should not
 * silently throw it away.
 */
export default function NameInput({ initialValue, selectStem = false, onCommit, onCancel }: NameInputProps) {
  const [value, setValue] = useState(initialValue);
  const inputRef = useRef<HTMLInputElement>(null);
  // Enter, Escape and blur can all fire for one gesture; only the first decides.
  const settled = useRef(false);

  useEffect(() => {
    const input = inputRef.current;
    if (!input) return;
    input.focus();
    const dot = selectStem ? initialValue.lastIndexOf(".") : -1;
    if (dot > 0) input.setSelectionRange(0, dot);
    else input.select();
  }, [initialValue, selectStem]);

  const commit = () => {
    if (settled.current) return;
    settled.current = true;
    const name = value.trim();
    // An empty field is a cancel, not an entry called "": the backend would
    // reject it anyway and the row would end up with nothing to name it.
    if (!name) onCancel();
    else onCommit(name);
  };

  const cancel = () => {
    if (settled.current) return;
    settled.current = true;
    onCancel();
  };

  return (
    <input
      ref={inputRef}
      value={value}
      placeholder="Name"
      aria-label="Name"
      onChange={(event) => setValue(event.target.value)}
      onKeyDown={(event) => {
        if (event.key === "Enter") { event.preventDefault(); commit(); }
        else if (event.key === "Escape") { event.preventDefault(); cancel(); }
      }}
      onBlur={commit}
      // The tree opens its menu on right-click; the field is not a target for it.
      onContextMenu={(event) => event.stopPropagation()}
      className="min-w-0 flex-1 rounded border border-[var(--accent)] bg-[var(--page)] px-1 py-0.5 text-[13px] text-[var(--text)] outline-none"
    />
  );
}

/**
 * A fresh row holding a {@link NameInput}, shown as the first child of the folder
 * being created into -- or of the root.
 *
 * It wears the same indent, icon and metrics as a real row on purpose: the new
 * entry reads as the file about to exist in the tree, not as a text box floating
 * over it.
 */
export function CreateNameRow({ depth, isDir, onCommit, onCancel }: { depth: number; isDir: boolean; onCommit: (name: string) => void; onCancel: () => void }) {
  const Icon = isDir ? Folder : FileText;
  return (
    <div style={{ paddingLeft: `${depth * 12 + 8}px` }} className="flex min-h-7 w-full items-center gap-1 rounded-md pr-2">
      <span className="size-4 shrink-0" />
      <div className="flex min-w-0 flex-1 items-center gap-1.5 py-1">
        <Icon size={14} className="shrink-0 text-[var(--muted)]" />
        <NameInput
          initialValue={isDir ? "new-folder" : "untitled.txt"}
          selectStem={!isDir}
          onCommit={onCommit}
          onCancel={onCancel}
        />
      </div>
    </div>
  );
}
