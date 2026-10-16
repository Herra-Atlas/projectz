import { useEffect, useMemo, useRef, useState, type ComponentProps, type ReactNode } from "react";
import { Check, Copy } from "lucide-react";
import type { ExtraProps } from "react-markdown";
import { findUrls } from "./urls";

/** The class a fence's info string arrives as, e.g. `language-html`. */
const LANGUAGE_PREFIX = "language-";

/** How long the button shows the tick after a successful copy. */
const COPIED_MS = 1500;

/** The hast `<pre>` element react-markdown hands an override. */
type HastElement = NonNullable<ExtraProps["node"]>;

/** The app's one notification stack, narrowed to the tones a one-off event uses. */
type Notify = (tone: "success" | "error", message: string) => void;

type CodeBlockProps = ComponentProps<"pre"> & ExtraProps & {
  /** Opens a link found in the code, in the panel's browser. */
  onOpenUrl: (url: string) => void;
  /** Reports a failed copy. Absent in the panel, where there is no stack in reach. */
  onNotify?: Notify;
};

/**
 * A fenced code block: a bar naming the fence's language, the code with the
 * addresses in it made clickable, and a copy control.
 *
 * **The box is drawn here, not by `MARKDOWN_STYLES`.** The block is now more than
 * a `pre` -- a bar above the code has to sit inside the same border -- so the
 * border, background and radius moved to this component and the shared rule kept
 * only the typography and the scrolling.
 *
 * **Why this takes over the `pre` element rather than decorating it.**
 * react-markdown has no notion of a link inside a fence -- a fence is literal by
 * definition -- so an address the model pasted into one is inert text, which is
 * exactly the case this exists to fix. Overriding `pre` is how the block's text,
 * its language and its control are claimed in one place.
 *
 * **Only URLs are linked; everything else stays literal.** No `rehype-raw` is in
 * the renderer, so a `<script>` a model writes is still the characters that spell
 * one -- this adds links to text, it does not start interpreting markup.
 *
 * **The text is read from the tree, not from `children`.** `children` is the
 * already-rendered `<code>` element; the tree underneath it is what the model
 * actually wrote, which is what the linkifier and the clipboard both want.
 */
export default function CodeBlock({ node, children, onOpenUrl, onNotify, ...rest }: CodeBlockProps) {
  const [copied, setCopied] = useState(false);
  const timerRef = useRef<number | null>(null);

  const code = useMemo(() => codeOf(node), [node]);
  const body = useMemo(() => (code === null ? null : linkify(code, onOpenUrl)), [code, onOpenUrl]);
  /** The fence's own language, shown as the block's title. `null` if it named none. */
  const language = useMemo(() => languageOf(node), [node]);

  // A tick left on screen after unmount would be a timer writing to a gone
  // component, so it goes with the block.
  useEffect(() => () => {
    if (timerRef.current !== null) window.clearTimeout(timerRef.current);
  }, []);

  const copy = () => {
    if (code === null) return;
    void navigator.clipboard.writeText(code).then(
      () => {
        setCopied(true);
        if (timerRef.current !== null) window.clearTimeout(timerRef.current);
        timerRef.current = window.setTimeout(() => setCopied(false), COPIED_MS);
      },
      () => onNotify?.("error", "Could not copy the code block."),
    );
  };

  // No tree to read: draw the block as it was handed over, with nothing to copy.
  // react-markdown always passes `node`, so this is the shape of a `pre` that did
  // not come from a fence.
  if (body === null) return <pre {...rest}>{children}</pre>;

  return (
    <div className="overflow-hidden rounded-lg border border-[var(--line)] bg-[var(--rail)]">
      {/* The block's own bar: the fence's language as the title, and the copy
          control opposite it. Above the code rather than floating over it, so
          neither sits on the first line and the bar holds still while a long
          block scrolls underneath. */}
      <div className="flex items-center gap-2 border-b border-[var(--line)] px-3 py-1.5">
        {/* Omitted rather than left empty when the fence named no language, so a
            plain block shows a bar with just the control on it. */}
        {language && <span className="min-w-0 truncate text-[11px] text-[var(--muted)]">{language}</span>}
        {/* The icon on its own, with no chip behind it: the bar already reads as
            the block's chrome, and a second box inside it would only compete.
            `ml-auto` holds it on the right whether or not there is a title. */}
        <button
          type="button"
          onClick={copy}
          aria-label={copied ? "Copied to clipboard" : "Copy code"}
          title={copied ? "Copied" : "Copy code"}
          className="ml-auto grid size-5 shrink-0 place-items-center rounded text-[var(--quiet)] transition-colors hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
        >
          {copied ? <Check size={13} /> : <Copy size={13} />}
        </button>
      </div>
      <pre {...rest}>
        {/* The language class rides along so a styled-by-language rule still finds it. */}
        <code className={language ? `language-${language}` : undefined}>{body}</code>
      </pre>
    </div>
  );
}

/**
 * The code's text with every URL in it drawn as a link.
 *
 * The anchors go where the caller's `onOpenUrl` goes -- the panel's browser, the
 * same place a link in prose opens -- so an address the model pasted is one click
 * from being seen without the block losing its shape. Text between the links is
 * passed through untouched, which is what keeps the code looking like code.
 */
function linkify(text: string, onOpenUrl: (url: string) => void): ReactNode[] {
  const parts: ReactNode[] = [];
  let cursor = 0;
  for (const { start, end, url } of findUrls(text)) {
    if (start > cursor) parts.push(text.slice(cursor, start));
    parts.push(
      <a key={start} href={url} onClick={(event) => { event.preventDefault(); onOpenUrl(url); }}>
        {text.slice(start, end)}
      </a>,
    );
    cursor = end;
  }
  if (cursor < text.length) parts.push(text.slice(cursor));
  return parts;
}

/** The `<code>` inside a `<pre>`, which is where a fence's text lives. */
function codeElement(node?: HastElement): HastElement | null {
  return node?.children.find((child): child is HastElement => child.type === "element" && child.tagName === "code") ?? null;
}

/** Everything written inside the fence, or `null` when there is no code element. */
function codeOf(node?: HastElement): string | null {
  const code = codeElement(node);
  return code ? textOf(code.children) : null;
}

/** The text under a node, so nested content is not silently dropped. */
function textOf(children: HastElement["children"]): string {
  return children
    .map((child) => (child.type === "text" ? child.value : child.type === "element" ? textOf(child.children) : ""))
    .join("");
}

/** The language a fence named, taken from its `language-…` class. */
function languageOf(node?: HastElement): string | null {
  const value = codeElement(node)?.properties?.className;
  if (!Array.isArray(value)) return null;
  const token = value.find((entry) => typeof entry === "string" && entry.startsWith(LANGUAGE_PREFIX));
  return typeof token === "string" ? token.slice(LANGUAGE_PREFIX.length) : null;
}
