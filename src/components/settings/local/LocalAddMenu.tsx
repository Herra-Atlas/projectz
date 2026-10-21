import { useEffect, useRef, useState } from "react";
import { ChevronRight, Download, Plus } from "lucide-react";

import { ANCHOR_ATTRIBUTE, FlashHighlight } from "../../SettingsSection";

/**
 * "Add models", with the two ways to add one behind it.
 *
 * There used to be one button because there was one answer -- pick a file. A
 * download is a second answer, and two buttons side by side make the page look
 * like it has two competing controls; a menu keeps one control and says which
 * kind of adding this is. The chevron is the only sign it opens anything, which
 * is why it is on the trigger and not on a separate icon.
 */
export function LocalAddMenu({
  anchor,
  onDownload,
  onAdd,
  label = "Add models",
  downloadLabel = "Download model",
  addLabel = "Add model",
}: {
  /** The settings-search anchor this control answers to. */
  anchor: string;
  onDownload: () => void;
  onAdd: () => void;
  label?: string;
  downloadLabel?: string;
  addLabel?: string;
}) {
  const [open, setOpen] = useState(false);
  const wrapper = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const close = (event: MouseEvent) => {
      if (!wrapper.current?.contains(event.target as Node)) setOpen(false);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", escape);
    };
  }, [open]);

  const item = "flex w-full min-h-9 items-center gap-2 px-3 text-left text-[12px] text-[var(--text)] transition-colors hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)]";

  return (
    <div ref={wrapper} className="relative">
      <button
        type="button"
        {...{ [ANCHOR_ATTRIBUTE]: anchor }}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((current) => !current)}
        className="inline-flex min-h-8 items-center gap-1.5 rounded-md border border-[var(--line)] px-2.5 text-[12px] font-medium text-[var(--text)] transition-colors hover:border-[var(--line-strong)] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
      >
        <FlashHighlight anchor={anchor} />
        <Plus size={13} />
        <span>{label}</span>
        <ChevronRight
          size={13}
          className={`transition-transform ${open ? "rotate-90" : ""}`}
        />
      </button>

      {open && (
        <div
          role="menu"
          className="absolute right-0 z-20 mt-1 w-48 overflow-hidden rounded-lg border border-[var(--line)] bg-[var(--panel)] py-1 shadow-xl"
        >
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              setOpen(false);
              onDownload();
            }}
            className={item}
          >
            <Download size={14} className="shrink-0 text-[var(--muted)]" />
            <span>{downloadLabel}</span>
          </button>
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              setOpen(false);
              onAdd();
            }}
            className={item}
          >
            <Plus size={14} className="shrink-0 text-[var(--muted)]" />
            <span>{addLabel}</span>
          </button>
        </div>
      )}
    </div>
  );
}
