import { useEffect, useRef, useState, type ReactNode } from "react";
import { ChevronDown, ChevronRight } from "lucide-react";

/**
 * One collapsible nav section of the settings sidebar.
 *
 * The parent row is a disclosure, not a page: its press opens and closes the
 * children under it, the way the Workspace entry behaves in the app's own
 * reference design. A group that contains the page being read keeps its accent
 * colour even while closed (`anyActive`), so the reader can always find which
 * section owns the page without opening anything.
 */
export function SettingsNavGroup({
  label,
  icon,
  anyActive,
  children,
}: {
  label: string;
  icon: ReactNode;
  /** True while a child's page is the one showing, open or not. */
  anyActive: boolean;
  children: ReactNode;
}) {
  // Closed when the modal opens: the whole rail folds to its parents, which is
  // what a list of sections reads as. A child reached from *outside* the rail
  // (a search result, a gear somewhere) opens its group on arrival -- the page
  // being visible is the proof the reader is there, and a closed group that
  // hides what a jump just requested is a wrong turn the rail cannot take back.
  const [open, setOpen] = useState(false);
  const everActive = useRef(false);
  useEffect(() => {
    if (!anyActive) {
      everActive.current = false;
      return;
    }
    if (everActive.current) return;
    everActive.current = true;
    setOpen(true);
  }, [anyActive]);
  return (
    <div>
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((current) => !current)}
        className={`flex min-h-9 w-full items-center gap-2.5 rounded-md px-2.5 text-left text-[13px] transition-colors focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)] ${
          anyActive
            ? "text-[var(--accent)]"
            : "text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]"
        }`}
      >
        {icon}
        {label}
        <span className="ml-auto grid shrink-0 place-items-center text-[var(--quiet)]" aria-hidden="true">
          {open ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
        </span>
      </button>
      {open && <div role="group" aria-label={label}>{children}</div>}
    </div>
  );
}

/**
 * The pressable style every plain (non-grouped) nav entry shares.
 *
 * Pulled out so the modal's ten single buttons and a group's children cannot
 * drift apart about what "selected" looks like. `dense` drops the height and
 * indents the row, which is what a child of a group is.
 */
export function navButtonClass(active: boolean, dense = false): string {
  // "Selected" is text colour only: the active entry was a translucent accent
  // wash under the label, and together with the accent text it read as a button
  // rather than as a read location -- the sibling single entries draw no
  // background when active either, so the rail says "where you are" in one
  // voice. Hover still raises, on both branches.
  return `flex ${dense ? "min-h-8 pl-9 pr-2.5" : "min-h-9 px-2.5"} w-full items-center gap-2.5 rounded-md text-left transition-colors focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)] ${
    active
      ? "text-[var(--accent)]"
      : "text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]"
  } ${dense ? "text-[12.5px]" : "text-[13px]"}`;
}
