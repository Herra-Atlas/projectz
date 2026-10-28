import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useNotificationHost } from "../features/notifications/notificationHost";

/**
 * How long the pointer must rest on an anchor before the card appears.
 *
 * Three seconds: a hint answers "what is this?", asked by stopping on something.
 * It is not a label that follows the pointer across a toolbar. Keyboard focus
 * still shows it at once -- landing on a control with Tab is deliberate, and
 * there is no hover to wait out.
 */
const HOVER_DELAY_MS = 3000;

/**
 * One themed hover card for every `title` in the app.
 *
 * **Why this is a listener and not a component per call site.** The native
 * `title` bubble is drawn by the OS: square-ish, delayed, and black-on-white (or
 * white-on-black) whatever the app's palette is, so it reads as a browser
 * artifact beside ProjectZ's own surfaces. A `<Tooltip>` wrapper would have to
 * be applied to a hundred elements, each one a chance to break a flex row, and
 * the next `title` somebody writes would be native again. This layer instead
 * watches the document for `[title]`, takes the hint away from the OS on hover
 * or focus, and draws it in ProjectZ's own card. New call sites need nothing.
 *
 * **The title is put back.** It is removed only while the card is visible, so
 * assistive technology and the next hover see the same attribute as before. If
 * the title changed under us (React re-rendered), the new one is kept.
 *
 * **Portalled into the open dialog when there is one.** `showModal()` promotes a
 * dialog to the top layer, where no z-index in the body can reach; a card drawn
 * into the body would sit behind the settings modal that raised it. The
 * notification host already tracks that dialog, so this reuses it rather than
 * opening a second channel.
 *
 * **Positioned from the anchor's rect.** The card is measured first (it is
 * rendered hidden for a frame), then flipped above when it would leave the
 * viewport and clamped to the edges. It is not clipped by the transcript or a
 * panel's scroll container, which an absolutely positioned child would be.
 */
export default function TooltipLayer() {
  const host = useNotificationHost();
  const [open, setOpen] = useState<{ hint: string; rect: DOMRect } | null>(null);
  const [at, setAt] = useState<{ left: number; top: number } | null>(null);
  const bubble = useRef<HTMLDivElement>(null);
  const anchor = useRef<HTMLElement | null>(null);
  const delay = useRef<number | undefined>(undefined);

  /** Put the original title back on the element we borrowed it from. */
  const restore = useCallback(() => {
    const element = anchor.current;
    anchor.current = null;
    if (!element) return;
    const hint = element.dataset.pzTooltip;
    if (!hint) return;
    // If React wrote a new title while the card was open, that one wins.
    if (!element.hasAttribute("title")) element.setAttribute("title", hint);
    delete element.dataset.pzTooltip;
  }, []);

  const hide = useCallback(() => {
    window.clearTimeout(delay.current);
    restore();
    setOpen(null);
    setAt(null);
  }, [restore]);

  /** Drop the card but keep the title borrowed: the pointer may still be on it. */
  const close = useCallback(() => {
    window.clearTimeout(delay.current);
    setOpen(null);
    setAt(null);
  }, []);

  const show = useCallback((element: HTMLElement, immediate = false) => {
    window.clearTimeout(delay.current);
    if (element === anchor.current) return;
    restore();
    if (element.tagName === "OPTION") return;
    const hint = element.getAttribute("title");
    if (!hint) return;
    element.dataset.pzTooltip = hint;
    element.removeAttribute("title");
    anchor.current = element;
    const rect = element.getBoundingClientRect();
    const draw = () => setOpen({ hint, rect });
    if (immediate) draw();
    else delay.current = window.setTimeout(draw, HOVER_DELAY_MS);
  }, [restore]);

  useEffect(() => {
    const onOver = (event: PointerEvent) => {
      const target = (event.target as Element | null)?.closest<HTMLElement>("[title]");
      if (target) show(target);
    };
    const onOut = (event: PointerEvent) => {
      const element = anchor.current;
      if (!element) return;
      const related = event.relatedTarget;
      if (related instanceof Node && element.contains(related)) return;
      hide();
    };
    const onFocusIn = (event: FocusEvent) => {
      const target = (event.target as Element | null)?.closest<HTMLElement>("[title]");
      if (target) show(target, true);
    };
    const onFocusOut = (event: FocusEvent) => {
      const element = anchor.current;
      if (!element) return;
      const related = event.relatedTarget;
      if (related instanceof Node && element.contains(related)) return;
      hide();
    };
    const dismiss = () => hide();
    const drop = () => close();

    document.addEventListener("pointerover", onOver, true);
    document.addEventListener("pointerout", onOut, true);
    document.addEventListener("focusin", onFocusIn, true);
    document.addEventListener("focusout", onFocusOut, true);
    document.addEventListener("pointerdown", drop, true);
    window.addEventListener("scroll", dismiss, true);
    window.addEventListener("resize", dismiss);
    return () => {
      document.removeEventListener("pointerover", onOver, true);
      document.removeEventListener("pointerout", onOut, true);
      document.removeEventListener("focusin", onFocusIn, true);
      document.removeEventListener("focusout", onFocusOut, true);
      document.removeEventListener("pointerdown", drop, true);
      window.removeEventListener("scroll", dismiss, true);
      window.removeEventListener("resize", dismiss);
    };
  }, [show, hide, close]);

  // A title can be written while the pointer is already inside: the copy button
  // becomes "Copied", a status row changes wording. React would put the title
  // back, and the OS would then draw its own bubble over ours after its delay.
  // Take it again as soon as it appears; the borrowed copy is what the card
  // shows, so the new wording is what the user reads.
  useEffect(() => {
    const observer = new MutationObserver((records) => {
      const element = anchor.current;
      if (!element) return;
      for (const record of records) {
        if (record.target !== element || record.attributeName !== "title") continue;
        const hint = element.getAttribute("title");
        if (!hint) continue;
        element.dataset.pzTooltip = hint;
        element.removeAttribute("title");
        setOpen((current) => (current ? { hint, rect: element.getBoundingClientRect() } : current));
      }
    });
    observer.observe(document.body, { attributes: true, attributeFilter: ["title"], subtree: true });
    return () => observer.disconnect();
  }, []);

  // Measure the card, then place it. One hidden frame is cheaper than guessing
  // at the wrapped height of a hint that may be a sentence.
  useLayoutEffect(() => {
    if (!open) return;
    const box = bubble.current?.getBoundingClientRect();
    if (!box) return;
    const gap = 8;
    const viewportWidth = window.innerWidth;
    const viewportHeight = window.innerHeight;
    const above = open.rect.bottom + gap + box.height > viewportHeight && open.rect.top - gap - box.height > 0;
    const left = Math.max(8, Math.min(open.rect.left, viewportWidth - box.width - 8));
    const top = above
      ? Math.max(8, open.rect.top - box.height - gap)
      : Math.max(8, Math.min(viewportHeight - box.height - 8, open.rect.bottom + gap));
    setAt({ left, top });
  }, [open]);

  useEffect(() => () => window.clearTimeout(delay.current), []);

  if (!open) return null;

  return createPortal(
    <div
      ref={bubble}
      role="tooltip"
      style={{ left: at?.left ?? 0, top: at?.top ?? 0, visibility: at ? "visible" : "hidden" }}
      className="pointer-events-none fixed z-[200] w-72 max-w-[calc(100vw_-_2rem)] whitespace-pre-wrap break-words rounded-xl bg-[var(--text)] px-3 py-2 text-left text-[11.5px] font-normal leading-4 text-[var(--page)] shadow-xl"
    >
      {open.hint}
    </div>,
    host ?? document.body,
  );
}
