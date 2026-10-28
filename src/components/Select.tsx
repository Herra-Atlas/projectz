import { useEffect, useId, useLayoutEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { Check, ChevronDown } from "lucide-react";
import { useNotificationHost } from "../features/notifications/notificationHost";

/**
 * One choice in a `Select`.
 *
 * `hint` is the small second thing a row can carry -- a provider's model count, a
 * note like "not installed" -- and becomes the row's tooltip, so the app's hover
 * card explains a long list without widening it.
 */
export type SelectOption = {
  value: string;
  label: string;
  hint?: string;
  disabled?: boolean;
};

type SelectProps = {
  value: string;
  options: readonly SelectOption[];
  onChange: (value: string) => void;
  /** Accessible name. The visible label belongs to the caller's form row. */
  label: string;
  disabled?: boolean;
  /** A leading glyph for the closed control, e.g. the provider icon. */
  icon?: ReactNode;
  /** Shown when nothing in `options` matches `value`. */
  placeholder?: string;
  /** `md` is a form field, `sm` a control sitting in a settings row. */
  size?: "sm" | "md";
  /** Width and flex sizing only. The component owns the surface and the border. */
  className?: string;
};

/** Keeps a dropdown of long model names readable beside a narrow trigger. */
const MIN_WIDTH = 180;
const MAX_WIDTH = 420;
const MAX_HEIGHT = 288;
const EDGE = 8;

/**
 * The app's dropdown.
 *
 * **Why not `<select>`.** A native list is drawn by Windows, not by this app: it
 * ignores every token, comes out white on a dark theme, and is a different
 * widget on every platform. It cannot show a second line of context or a check
 * beside the current choice either. This is the same popup as the files menu, so
 * a dropdown and a right-click menu are one visual language.
 *
 * **Anchored and portalled.** The menu is measured from the trigger's rect,
 * opened downward with a flip upward when there is no room, and drawn into the
 * notification host -- the open dialog -- because `showModal()` puts its content
 * in the top layer and nothing in the body can paint over it. Clipped by a
 * scrolling panel is the other failure this avoids.
 *
 * **Focus stays on the trigger.** The button keeps the caret and drives the list
 * with the arrow keys while the menu is a `listbox` described by
 * `aria-activedescendant`; nothing inside the portal is focusable, so Tab always
 * leaves the control rather than landing in a menu.
 *
 * The closed state is a button rather than a styled `<select>` on purpose: the
 * value is truncated in the middle, the caret space belongs to the chevron, and
 * `Enter`/`Space` open the list instead of submitting the form around it.
 */
export default function Select({
  value,
  options,
  onChange,
  label,
  disabled = false,
  icon,
  placeholder,
  size = "md",
  className = "",
}: SelectProps) {
  const host = useNotificationHost();
  const id = useId();
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [anchor, setAnchor] = useState<DOMRect | null>(null);
  const [placed, setPlaced] = useState<{ top: number; left: number } | null>(null);
  const [activeIndex, setActiveIndex] = useState(0);

  const selectedIndex = options.findIndex((option) => option.value === value);
  const selected = selectedIndex >= 0 ? options[selectedIndex] : undefined;
  /** Where the arrows start when nothing is chosen: the first row that can be. */
  const firstEnabledIndex = (() => {
    if (selectedIndex >= 0) return selectedIndex;
    const first = options.findIndex((option) => !option.disabled);
    return first >= 0 ? first : 0;
  })();

  /** Opens the list with `index` as the keyboard position, measured and placed. */
  const openMenu = (index: number) => {
    const rect = triggerRef.current?.getBoundingClientRect();
    if (!rect) return;
    setAnchor(rect);
    setPlaced(null);
    setActiveIndex(Math.max(0, index));
    setOpen(true);
  };

  const choose = (option: SelectOption) => {
    if (option.disabled) return;
    onChange(option.value);
    setOpen(false);
    triggerRef.current?.focus();
  };

  /** Moves to the next enabled row, wrapping, so a disabled row is skipped. */
  const move = (direction: 1 | -1) => {
    setActiveIndex((current) => {
      if (options.length === 0) return 0;
      for (let step = 1; step <= options.length; step += 1) {
        const next = ((current + direction * step) % options.length + options.length) % options.length;
        if (!options[next].disabled) return next;
      }
      return current;
    });
  };

  const onTriggerKeyDown = (event: ReactKeyboardEvent<HTMLButtonElement>) => {
    if (disabled) return;
    if (!open) {
      // The arrows open with the current choice highlighted, the way a native
      // list does; opening with the first row would lose the reader's place in
      // a long list of languages or models.
      if (event.key === "ArrowDown" || event.key === "ArrowUp" || event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        openMenu(firstEnabledIndex);
      }
      return;
    }
    switch (event.key) {
      case "ArrowDown":
        event.preventDefault();
        move(1);
        break;
      case "ArrowUp":
        event.preventDefault();
        move(-1);
        break;
      case "Home":
        event.preventDefault();
        setActiveIndex(options.findIndex((option) => !option.disabled));
        break;
      case "End":
        event.preventDefault();
        setActiveIndex(options.map((option) => !option.disabled).lastIndexOf(true));
        break;
      case "Enter":
      case " ":
        event.preventDefault();
        if (options[activeIndex]) choose(options[activeIndex]);
        break;
      case "Escape":
        event.preventDefault();
        setOpen(false);
        break;
      case "Tab":
        // Leaving the control is a dismissal, not a selection: a menu that
        // stayed open behind the next field would be floating over it.
        setOpen(false);
        break;
      default:
        break;
    }
  };

  // Dismissal. Anything that moves the page closes the list rather than leaving
  // it hanging where the trigger used to be.
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target as Node | null;
      if (!target) return;
      if (menuRef.current?.contains(target) || triggerRef.current?.contains(target)) return;
      setOpen(false);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      setOpen(false);
      triggerRef.current?.focus();
    };
    const onScroll = (event: Event) => {
      // The menu's own scrolling is not a reason to close it.
      if (menuRef.current?.contains(event.target as Node)) return;
      setOpen(false);
    };
    const onResize = () => setOpen(false);

    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", onResize);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", onResize);
    };
  }, [open]);

  // Place once the menu has a measured height: a flip upward is a decision that
  // needs the real height, and guessing it puts a long list off-screen.
  useLayoutEffect(() => {
    if (!open || !anchor) return;
    const box = menuRef.current?.getBoundingClientRect();
    if (!box) return;
    const below = anchor.bottom + 4;
    const above = anchor.top - 4 - box.height;
    const flip = below + box.height > window.innerHeight - EDGE && above > EDGE;
    const left = Math.max(EDGE, Math.min(anchor.left, window.innerWidth - box.width - EDGE));
    setPlaced({ top: flip ? above : below, left });
  }, [open, anchor, options.length]);

  // A long list opens with the current choice in view, not at the top.
  useEffect(() => {
    if (!open) return;
    const current = menuRef.current?.querySelector<HTMLElement>('[aria-selected="true"]');
    current?.scrollIntoView({ block: "nearest" });
  }, [open]);

  const width = anchor ? Math.min(Math.max(anchor.width, MIN_WIDTH), MAX_WIDTH) : MIN_WIDTH;
  const triggerSize = size === "sm" ? "min-h-8 rounded-md px-2 text-[12px]" : "min-h-9 rounded-lg px-2.5 text-[13px]";

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={label}
        aria-activedescendant={open ? `${id}-option-${activeIndex}` : undefined}
        disabled={disabled}
        onClick={() => (open ? setOpen(false) : openMenu(firstEnabledIndex))}
        onKeyDown={onTriggerKeyDown}
        className={`flex items-center gap-2 border border-[var(--line)] bg-[var(--rail)] text-left text-[var(--text)] outline-none transition-colors hover:border-[var(--line-strong)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)] disabled:pointer-events-none disabled:opacity-40 ${triggerSize} ${className}`}
      >
        {icon && <span className="shrink-0 text-[var(--muted)]">{icon}</span>}
        <span className="min-w-0 flex-1 truncate">{selected?.label ?? placeholder ?? value ?? ""}</span>
        <ChevronDown size={14} className={`shrink-0 text-[var(--quiet)] transition-transform duration-150 ${open ? "rotate-180" : ""}`} />
      </button>

      {open && anchor && createPortal(
        <div
          ref={menuRef}
          role="listbox"
          aria-label={label}
          style={{
            position: "fixed",
            width,
            maxHeight: MAX_HEIGHT,
            left: placed?.left ?? anchor.left,
            top: placed?.top ?? anchor.bottom + 4,
            // Hidden for the one paint before placement lands, so the menu never
            // flashes at the unclamped corner and then jumps.
            visibility: placed ? undefined : "hidden",
          }}
          className="z-[140] overflow-y-auto overscroll-contain rounded-lg border border-[var(--line)] bg-[var(--rail)] p-1 shadow-2xl"
        >
          {options.map((option, index) => {
            const isSelected = option.value === value;
            return (
              <button
                key={option.value}
                id={`${id}-option-${index}`}
                type="button"
                role="option"
                aria-selected={isSelected}
                disabled={option.disabled}
                // `pointermove`, not `mouseenter`: the keyboard position follows
                // the pointer only while it is genuinely moving over a row.
                onPointerMove={() => setActiveIndex(index)}
                onClick={() => choose(option)}
                title={option.hint}
                className={`flex min-h-8 w-full items-center gap-2 rounded-md px-2 text-left text-xs transition-colors focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)] disabled:opacity-40 ${index === activeIndex ? "bg-[var(--raised)]" : "hover:bg-[var(--raised)]"} ${isSelected ? "text-[var(--text)]" : "text-[var(--muted)]"}`}
              >
                <span className="min-w-0 flex-1 truncate">{option.label}</span>
                {option.hint && <span className="shrink-0 text-[10.5px] text-[var(--quiet)]">{option.hint}</span>}
                {isSelected && <Check size={13} className="shrink-0 text-[var(--accent)]" />}
              </button>
            );
          })}
        </div>,
        host ?? document.body,
      )}
    </>
  );
}
