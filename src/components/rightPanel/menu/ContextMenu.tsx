import { Fragment, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { LucideIcon } from "lucide-react";

/**
 * One row of a context menu.
 *
 * `separated` draws the hairline above the row, so a caller can group its
 * actions -- creates, then edit, then the destructive one -- without threading
 * divider elements between the items itself.
 */
export type ContextMenuItem = {
  id: string;
  label: string;
  icon: LucideIcon;
  onSelect: () => void;
  /** The destructive action, drawn in the danger colour. */
  tone?: "danger";
  /** A rule above this row, splitting it from the group before it. */
  separated?: boolean;
};

type ContextMenuProps = {
  /** Where the pointer asked for the menu, in viewport coordinates. */
  point: { x: number; y: number };
  /** Accessible name for the menu, e.g. "File actions". */
  label: string;
  items: ContextMenuItem[];
  onClose: () => void;
};

/** Meters matching the session menu in `Sidebar`, so both read as one widget. */
const WIDTH = 200;
const ROW_HEIGHT = 32;
const EDGE = 8;

/**
 * A menu drawn at the pointer, shared by every right-click surface in the Files
 * view: a file row, a folder row, and the blank space under the tree.
 *
 * **Modelled on the session menu**, not reinvented: the same `--rail` surface,
 * the same 32px rows, the same hover and focus treatment. A menu that looked
 * different from the one in the sidebar would read as a second visual language
 * for the same gesture.
 *
 * **Portalled to `body`.** The tree scrolls inside an `overflow` column, so a
 * menu drawn in place would be clipped the moment it neared the panel's edge.
 *
 * It owns only its own placement and dismissal; what each item does, and what
 * the menu is *about*, belong to the caller -- which keeps one menu for the whole
 * view instead of one per row.
 */
export default function ContextMenu({ point, label, items, onClose }: ContextMenuProps) {
  const ref = useRef<HTMLDivElement>(null);
  const [placed, setPlaced] = useState<{ top: number; left: number } | null>(null);

  // Dismiss on the next click anywhere else, and on Escape. Nothing here holds a
  // selection, so closing is the whole of the menu's own state.
  useEffect(() => {
    const dismissOutside = (event: PointerEvent) => {
      if (ref.current && !ref.current.contains(event.target as Node)) onClose();
    };
    const dismissEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    document.addEventListener("pointerdown", dismissOutside);
    document.addEventListener("keydown", dismissEscape);
    return () => {
      document.removeEventListener("pointerdown", dismissOutside);
      document.removeEventListener("keydown", dismissEscape);
    };
  }, [onClose]);

  // Clamp into the viewport once the real height is known. A fixed menu opened
  // past an edge cannot be scrolled back into view, so the pointer position is a
  // request and the clamped position is what gets drawn.
  useLayoutEffect(() => {
    const height = ref.current?.offsetHeight ?? items.length * ROW_HEIGHT + EDGE;
    const left = Math.max(EDGE, Math.min(point.x, window.innerWidth - WIDTH - EDGE));
    const top = Math.max(EDGE, Math.min(point.y, window.innerHeight - height - EDGE));
    setPlaced({ top, left });
  }, [point, items.length]);

  return createPortal(
    <div
      ref={ref}
      role="menu"
      aria-label={label}
      style={{
        position: "fixed",
        width: WIDTH,
        top: placed?.top ?? point.y,
        left: placed?.left ?? point.x,
        // Hidden for the single paint before the clamp lands, so the menu never
        // flashes at an unclamped position and then jumps to the clamped one.
        visibility: placed ? undefined : "hidden",
      }}
      className="z-[100] rounded-lg border border-[var(--line)] bg-[var(--rail)] p-1 shadow-2xl"
    >
      {items.map((item) => (
        <Fragment key={item.id}>
          {item.separated && <div className="my-1 border-t border-[var(--line)]" />}
          <button
            type="button"
            role="menuitem"
            onClick={() => { item.onSelect(); onClose(); }}
            className={`flex min-h-8 w-full items-center gap-2.5 rounded-md px-2 text-left text-xs hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)] ${item.tone === "danger" ? "text-[var(--danger)]" : "text-[var(--text)]"}`}
          >
            <item.icon size={14} className={item.tone === "danger" ? undefined : "text-[var(--muted)]"} />
            {item.label}
          </button>
        </Fragment>
      ))}
    </div>,
    document.body,
  );
}
