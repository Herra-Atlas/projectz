import { useEffect, useRef, useState, type ReactNode } from "react";
import HelpMark from "./HelpMark";

/** Shared primitives for the settings pages.
    A section is a titled group of rows without a panel: no background and no
    outer border, because a settings page is a column of values rather than a
    stack of cards. A **hairline between rows** stays, because that is what lets
    the eye separate "Title generation" from the row under it without drawing a
    box around both. Spacing and that line carry the grouping. */

type SettingsSectionProps = { title: string; children: ReactNode };

export function SettingsSection({ title, children }: SettingsSectionProps) {
  return (
    // A category is separated by a **hairline that fades to the right**, not a hard bar
    // across the page: the boundary is visible where the heading is and dissolves where
    // the eye is no longer reading, so it marks the group without drawing a strike
    // through the layout. Row dividers stay a flat 1px `--line`, so "another setting" and
    // "another category" still read differently.
    // The rule is hidden on a section that directly follows the title band, by
    // `App.css` (`[data-settings-title] + section`) rather than by `first:`, because the
    // band and the first section are siblings and `first-child` would never match. The
    // band already ends in a hairline; a second rule 30px below it is the "two lines" that
    // made General, Appearance and Taste look framed twice.
    <section className="settings-section mt-8">
      <div className="settings-section-rule h-px w-full bg-[linear-gradient(to_right,var(--line-strong),var(--line)_45%,transparent)]" />
      {/* Aligned with the row labels, not the page edge: without the panel there is no
          border for the heading to sit on, and a heading at 0 beside rows at 16 read as
          a misalignment rather than as a hierarchy. */}
      <h3 className="px-4 pb-2 pt-4 text-[12px] font-semibold tracking-tight text-[var(--muted)]">{title}</h3>
      {/* No wrapper surface, only the rule between rows. It also cannot clip a
          popover: a model picker opening downwards from a row, or a preview card
          anchored inside one, used to be cut off by an `overflow-hidden` panel.
          With no panel there is nothing to clip. */}
      <div className="divide-y divide-[var(--line)]">{children}</div>
    </section>
  );
}

/**
 * The title band every settings page opens with.
 *
 * **One shape, everywhere.** The pages had grown four variants of this row -- with a
 * hairline and without, `mb-6 pb-5` and `mb-5`, a description under the heading or not --
 * so moving from Preferences to General moved the first section up or down for no reason a
 * reader could see. The hairline and the 20px of breathing room are what makes the heading
 * sit *on* the page rather than float over the first section, so every page uses this and
 * a page that needs a control in the corner puts it in the row this renders.
 *
 * The extra space below is deliberate: `SettingsSection` is `mt-8 first:mt-0`, and a page
 * whose first section had no margin would start flush against this band.
 */
export function SettingsPageTitle({ children }: { children: ReactNode }) {
  return (
    <div className="relative mb-6 border-b border-[var(--line)] pb-5" data-settings-title="">
      <div className="flex flex-wrap items-center justify-between gap-3">{children}</div>
      {/* Recolours the band's hairline when a search result lands here. The band is the
          unambiguous "you are on this page" mark: two adjacent pages are one hairline
          apart, so a dimmed neighbour has nothing to dim *to*. */}
      <FlashHighlight anchor="title" />
    </div>
  );
}

/**
 * Dims everything beside a search result while its mark is up.
 *
 * Wraps the page body rather than the title band, because the dim has to reach the rows:
 * a class on the band alone would fade the heading and leave the settings at full
 * strength. The class is removed on the same timer the mark uses, so nothing dims outside
 * a search and nothing stays dim afterwards.
 *
 * Only elements carrying `data-settings-markable` are dimmed -- every row, and the title
 * band -- not every descendant, so a section's heading and its dividers keep the contrast
 * they were drawn with.
 */
export function SettingsSearchScope({ children }: { children: ReactNode }) {
  const [searching, setSearching] = useState(false);
  useEffect(() => {
    let timer: number | undefined;
    const handler = () => {
      window.clearTimeout(timer);
      setSearching(true);
      timer = window.setTimeout(() => setSearching(false), HIGHLIGHT_LIFETIME_MS);
    };
    window.addEventListener(SETTINGS_HIGHLIGHT_EVENT, handler);
    return () => {
      window.removeEventListener(SETTINGS_HIGHLIGHT_EVENT, handler);
      window.clearTimeout(timer);
    };
  }, []);
  return <div className={searching ? "settings-search-mode" : undefined}>{children}</div>;
}

/** The event name a search result announces itself with. One constant, so the emitter and
 *  the listener cannot disagree about what they are listening for. */
export const SETTINGS_HIGHLIGHT_EVENT = "settings:highlight";

/** How long a search result's mark lasts. Kept in step with `App.css`. */
const HIGHLIGHT_LIFETIME_MS = 2000;

/** The attribute a findable row carries, named in one place for the index and the mark. */
export const ANCHOR_ATTRIBUTE = "data-settings-anchor";

/**
 * The attribute that makes an element recede while a result is marked.
 *
 * Separate from `ANCHOR_ATTRIBUTE` because the two answer different questions: the anchor
 * says "search can point here", and this says "dim me when it points elsewhere". A row is
 * both; a section heading is neither, and the divider between rows is not marked at all --
 * dimming the rules too would make the page look disabled rather than focused.
 */
export const MARKABLE_ATTRIBUTE = "data-settings-markable";

/**
 * Marks *this element* when a search result names the anchor it carries.
 *
 * **A DOM mark, not a prop.** The pages are already written and the rows they draw are the
 * ones search points at; threading a `highlighted` prop from the modal through the page, the
 * section and the row would put a piece of *search's* state into a component that is
 * otherwise only about drawing a value.
 *
 * **Each element listens for itself.** The first version was one highlighter per page that
 * searched the document for the anchor -- and it never found a row carrying it, because the
 * `target` it was given was `"title"` and a row is not a title. The element already knows
 * its own anchor (`anchor` on a row, `"title"` on the band), so the comparison is a string
 * against itself and there is no document query to get wrong.
 *
 * The attribute is the whole mechanism: the stylesheet recolours the text inside
 * `[data-highlight-mark]` and fades it back, which is why removal is a timer rather than a
 * re-render. It is attached to the *row*, not to a wrapper, so the colour lands on the words
 * the reader is being pointed at.
 */
export function FlashHighlight({ anchor }: { anchor: string }) {
  const ref = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    let timer: number | undefined;
    const onHighlight = (event: Event) => {
      const wanted = (event as CustomEvent<{ anchor: string }>).detail?.anchor;
      // The listener may live inside the element it marks (a row) or be the element's own
      // first child (a button, a heading). `closest` walks outward from the span, so it
      // finds the row in the first case and the button in the second -- and if the span is
      // itself the only thing there, it marks itself, which still tints nothing.
      const host =
        anchor === "title"
          ? ref.current?.closest("[data-settings-title]")
          : ref.current?.closest(`[${ANCHOR_ATTRIBUTE}]`) ?? ref.current;
      if (!host) return;
      // Every result lights its page's band; only the named row lights itself.
      if (anchor !== "title" && wanted !== anchor) return;
      // Restarting means removing first: an attribute that is already set does not re-run
      // the animation, so a repeat visit to the same row would otherwise not flash again.
      host.removeAttribute("data-highlight-mark");
      // A frame between removal and re-add lets the browser observe the change; without it
      // the two writes in one task collapse and nothing animates.
      window.requestAnimationFrame(() => host.setAttribute("data-highlight-mark", ""));
      window.clearTimeout(timer);
      timer = window.setTimeout(() => host.removeAttribute("data-highlight-mark"), HIGHLIGHT_LIFETIME_MS);
    };
    window.addEventListener(SETTINGS_HIGHLIGHT_EVENT, onHighlight);
    return () => {
      window.removeEventListener(SETTINGS_HIGHLIGHT_EVENT, onHighlight);
      window.clearTimeout(timer);
    };
  }, [anchor]);
  return <span ref={ref} hidden />;
}

type SettingRowProps = {
  label: string;
  /**
   * The name search finds this row by, and marks it with.
   *
   * Optional because not every row is findable -- an arithmetic one like "Next run" is
   * found through its page. Present means the row is in `settingsIndex.ts`; the two are
   * kept in step by hand, and a row with an anchor that is not in the index is simply
   * unsearchable, which is the quiet failure rather than the loud one.
   */
  anchor?: string;
  /** One short line under the label. For anything longer, use `hint`. */
  description?: string;
  /**
   * The long explanation, behind a `?` beside the label.
   *
   * The same affordance the job editor uses, for the same reason: a paragraph under every
   * row buries the rows it is describing, and the explanation is only wanted when it is
   * asked for. A row with three sentences of prose under it is a row nobody reads.
   */
  hint?: string;
  /** Full-width controls, such as a textarea, sit under the label instead of beside it. */
  stacked?: boolean;
  control?: ReactNode;
};

export function SettingRow({ label, anchor, description, hint, stacked, control }: SettingRowProps) {
  return (
    <div
      {...(anchor ? { [ANCHOR_ATTRIBUTE]: anchor } : {})}
      // Every row dims, anchored or not: the marked row can be anywhere in the list, and a
      // list where only some rows fade tells the reader nothing. Section headings and the
      // rules between rows stay at contrast -- they are the page's structure, not its
      // content, and dimming them makes the window look disabled.
      {...{ [MARKABLE_ATTRIBUTE]: "" }}
      className={`flex gap-6 px-4 py-3.5 ${stacked ? "flex-col items-stretch" : "min-h-[58px] items-center justify-between"}`}
    >
      {anchor && <FlashHighlight anchor={anchor} />}
      <div className="min-w-0">
        <p className="flex items-center gap-1.5 text-[13px]">
          <span>{label}</span>
          {hint && <HelpMark hint={hint} />}
        </p>
        {description && <p className="mt-0.5 max-w-md text-[12px] leading-5 text-[var(--muted)]">{description}</p>}
      </div>
      {control && <div className={stacked ? "mt-3" : "shrink-0"}>{control}</div>}
    </div>
  );
}

export function Toggle({ checked, onChange, label }: { checked: boolean; onChange: (next: boolean) => void; label: string }) {
  return (
    <button type="button" role="switch" aria-label={label} aria-checked={checked} onClick={() => onChange(!checked)} className={`relative inline-flex h-[22px] w-[38px] items-center rounded-full transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)] ${checked ? "bg-[var(--accent)]" : "bg-[var(--raised)]"}`}>
      <span className={`inline-block size-4 rounded-full bg-[var(--page)] transition-transform ${checked ? "translate-x-[20px]" : "translate-x-0.5"}`} />
    </button>
  );
}

type SegmentedProps<T extends string> = { value: T; options: readonly T[]; onChange: (next: T) => void; label: string };

/** Mutually exclusive options shown as one control, so the current choice is visible
    without opening anything. */
export function Segmented<T extends string>({ value, options, onChange, label }: SegmentedProps<T>) {
  return (
    <div role="group" aria-label={label} className="inline-flex shrink-0 gap-0.5 rounded-lg bg-[var(--raised)] p-0.5">
      {options.map((option) => <button key={option} type="button" aria-pressed={value === option} onClick={() => onChange(option)} className={`min-h-7 rounded-[6px] px-2.5 text-[12px] transition-colors focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)] ${value === option ? "bg-[var(--panel)] text-[var(--text)] shadow-sm" : "text-[var(--muted)] hover:text-[var(--text)]"}`}>{option}</button>)}
    </div>
  );
}
