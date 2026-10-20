/**
 * Reading what is on the page, and drawing the answers beside it.
 *
 * # Why this is a plain script
 *
 * A content script cannot `import` an extension module without exposing it as a
 * web-accessible resource, which would let any page fetch it. The constants it shares with
 * `lib/settings.js` are storage keys, and it reads the same `chrome.storage.local` record
 * that file defines -- so the two agree by reading one thing, not by importing one file.
 *
 * # What it decides, and what it does not
 *
 * Whether a block is *worth answering* is not decided here. This decides which pages may be
 * read at all, what text is on them, and when they are worth looking at -- browser questions the
 * app has no view of -- and the app decides everything after that. The gate, the model and the
 * budget all live there, so the rules about what deserves an answer exist once.
 *
 * The one judgement made here is **"this page has not changed since I last sent it"**, because
 * this is the only place that can know it: the app forgets every look on purpose, and a request
 * is the whole cost of a look. It is a fact about this document, not a memory about the text.
 *
 * # What it will not read
 *
 * An allowlist of schemes, and nothing else: `http`, `https`, and `file` when the user has asked
 * for it in two places -- this extension's own setting, and Chrome's "Allow access to file URLs"
 * for it. `chrome://`, `chrome-extension://` (this extension's own pages included), `devtools://`
 * and the rest are not content, and several of them are the browser's own settings.
 *
 * # The whole page, and each block once
 *
 * The page is read in full rather than only where the reader is looking: a question below the fold
 * is still a question, and a list whose numbers depend on the scroll position is a list that means
 * something different by the time the answer comes back. What is *not* sent is a block this page has
 * already been answered about -- the answers are on screen, and a second look at the same text is
 * the same request bought twice.
 *
 * # Why the cards are drawn in a shadow root
 *
 * A page's own CSS would otherwise reach them: a quiz site styling every `div`, every
 * `button` and every `p` would style the card too, differently on every site, and a card
 * that looks broken on one site is worse than no card. A closed-ish shadow root with its
 * own stylesheet is the one way to be sure the card looks the same everywhere -- and it
 * also keeps the card out of the page's own queries, so the page's own scripts cannot see
 * it, restyle it or read it.
 *
 * The inline style gives that up deliberately: an aside written beside the text has to live in
 * the page's own tree to sit in the page's own flow, so its styles are declared inline (which
 * every selector except `!important` loses to), it is marked with an attribute, and it is put
 * back if the page removes it. The trade is the price of the shape.
 */

(() => {
  /** The same defaults `lib/settings.js` carries, and the same storage record. */
  const DEFAULTS = {
    enabled: false,
    display: true,
    auto: true,
    settle: 900,
    files: false,
    limits: { minChars: 6, maxAskChars: 1200 },
    maxBlocks: 60,
    maxBlockChars: 600,
    cardStyle: "card",
    card: "auto",
    opacity: 0.88,
    hideAfter: 0,
    sites: {},
  };

  const HOST_ID = "projectz-assist-cards";

  /**
   * What the app's floor and cut fall back to before it has told us its own.
   *
   * Not the authority -- the app is, and these are replaced by its numbers on the first frame
   * (`state.limits`). They exist for the second between the page loading and the socket
   * answering, and they are the app's defaults rather than numbers of this file's own: a
   * floor invented here is a second answer to a question the user can only change in Settings.
   */
  const FALLBACK_MIN_CHARS = 6;
  const FALLBACK_ASK_CHARS = 1200;
  /** Most blocks one look may send, and most text one look may carry, in characters. */
  const FALLBACK_MAX_BLOCKS = 60;
  const MAX_FRAME_CHARS = 20000;
  /** How long the page must be still before it is read, unless the options page says otherwise. */
  const DEFAULT_SETTLE_MS = 900;
  /** How long after typing, clicking or scrolling a read is held off. */
  const ACTIVE_MS = 1500;
  /** How long to wait for an answer before giving up on the "reading" state. */
  const ANSWER_TIMEOUT_MS = 45000;
  /**
   * How long to stop looking after the app says it cannot answer.
   *
   * A page that was read and refused is not worth reading again a second later -- the
   * answer will be the same -- and the first real session showed exactly that: a page with
   * nothing on it that assist could use was re-sent every few seconds for as long as it was
   * open. The app remembers nothing between looks (on purpose), so when it cannot answer at
   * all, the waiting has to happen here.
   */
  const COOLDOWN_MS = 60000;
  /** Shorter, because a failed request may well work the next time. */
  const FAILED_COOLDOWN_MS = 15000;
  const CARD_WIDTH = 244;
  const CARD_MAX_HEIGHT = 200;

  /** Elements whose text is never page content. */
  const SKIP = new Set([
    "SCRIPT", "STYLE", "NOSCRIPT", "SVG", "CANVAS", "IFRAME", "TEXTAREA", "INPUT",
    // A button's label is chrome: "Edit", "Share", "Show more". On a page built out of inline
    // elements the whole toolbar can otherwise arrive as one block of its own.
    "BUTTON",
    "SELECT", "OPTION", "VIDEO", "AUDIO", "MAP", "OBJECT", "EMBED",
  ]);
  /**
   * Elements that do not disqualify their parent from being one block.
   *
   * This is what decides where a block *ends*: a paragraph holding bold text and a link is
   * still one paragraph, while one holding another paragraph is two blocks -- and treating
   * them the other way round would send the same words twice, once alone and once with
   * everything around it.
   */
  const INLINE = new Set([
    "A", "B", "I", "EM", "STRONG", "SPAN", "SMALL", "MARK", "SUB", "SUP", "U", "S", "BR",
    "WBR", "ABBR", "CITE", "Q", "TIME", "KBD", "SAMP", "VAR", "BDO", "BDI", "FONT", "IMG",
  ]);

  let state = { ...DEFAULTS };
  /** The blocks of the last look: index -> the element they came from. */
  let blocks = new Map();
  /** The same blocks by *name*: text hash -> element. See `blockFor`. */
  let keys = new Map();
  /**
   * The names of the blocks this page has already been answered about.
   *
   * **The one thing that stops the same question costing twice.** A page that changes -- a
   * framework re-rendering a list, a score ticking over, the reader typing -- is a page whose text
   * had changed, so the unchanged-page check cannot fire and every look would send the whole thing
   * again: the same code sample, the same quiz, the same answers, asked about over and over. Named
   * by *text* rather than by position, because the position is what moves.
   */
  const answered = new Set();
  /** Drawn cards: index -> the element. */
  const cards = new Map();
  let lastActivity = 0;
  let settleTimer = 0;
  let holdTimer = 0;
  let reading = 0;
  let raf = 0;
  /** Before this moment, no look happens: the app has nothing to say about this page yet. */
  let cooldownUntil = 0;
  /** The layer the cards live in, created the first time one is drawn. */
  let layer = null;
  /** The style the answers on screen were drawn with, so a change can take them with it. */
  let drawnWith = "";
  /**
   * The fingerprint of the last text sent, or -1 for "nothing has been sent".
   *
   * The one thing this page remembers between looks, and it remembers only *that*, never what
   * was asked or answered: a page whose text is unchanged is not sent again, and the app's own
   * rule -- nothing is remembered between looks -- is untouched, because this is a fact about
   * the browser's own document rather than a judgement about the text. `-1` cannot collide
   * with a real fingerprint, which is unsigned.
   */
  let lastSent = -1;

  // -------------------------------------------------------------------------------------
  // Settings, mirrored from storage
  // -------------------------------------------------------------------------------------

  async function loadState() {
    try {
      const stored = await chrome.storage.local.get(DEFAULTS);
      state = { ...DEFAULTS, ...stored };
    } catch {
      // A page whose extension context was invalidated (the extension was reloaded while
      // this tab stayed open) throws here. Staying on the defaults means the page simply
      // stops being read, which is the right failure.
      state = { ...DEFAULTS };
    }
  }

  const host = (url) => {
    try {
      return new URL(url).hostname;
    } catch {
      return "";
    }
  };

  /**
   * Whether this page is one assist may read at all.
   *
   * A scheme allowlist rather than "not a browser page", because everything else is longer
   * than it looks: `chrome://`, `chrome-extension://` (this extension's own options page
   * included), `devtools://`, `view-source:` and a PDF viewer are not content, and several of
   * them are the browser's own settings. `file:` is read only when the user has asked for it
   * in this extension *and* given Chrome the file-access grant -- without the second, this
   * script is not even injected into the page.
   */
  const readable = () => {
    if (location.protocol === "http:" || location.protocol === "https:") return true;
    return location.protocol === "file:" && state.files === true;
  };

  /**
   * Whether this page should be read, as things stand right now.
   *
   * A `file:` page has no hostname, so a site exception cannot apply to it and only the scheme
   * decides -- and an empty host must never be looked up in `sites`, or one rule about one
   * site would silence every local file.
   */
  const allowedHere = () => {
    if (!readable()) return false;
    const name = host(location.href);
    return !name || state.sites[name] !== "off";
  };

  /** How long the page must be still, as the options page has it. */
  const settleMs = () => Number(state.settle) || DEFAULT_SETTLE_MS;

  /** How long a block must be before the app would look at it. */
  const minChars = () => Number(state.limits && state.limits.minChars) || FALLBACK_MIN_CHARS;

  /**
   * How much of a block may cross the wire.
   *
   * The smaller of the two bounds, because they answer different questions: the app's cut is
   * what it will read, and this extension's is what its own wire budget allows. Sending more
   * than either is characters that are paid for and then ignored.
   */
  const blockCut = () =>
    Math.min(
      Number(state.limits && state.limits.maxAskChars) || FALLBACK_ASK_CHARS,
      Number(state.maxBlockChars) || 600
    );

  /** Most blocks one look may send. */
  const maxBlocks = () => Number(state.maxBlocks) || FALLBACK_MAX_BLOCKS;

  /** How an answer is drawn: a card beside the text, or an aside after it. */
  const cardStyle = () => (state.cardStyle === "inline" ? "inline" : "card");

  /**
   * How solid a card is.
   *
   * Clamped here as well as in the options page, because a record edited by hand is a card that
   * cannot be read: under about 0.6 the words stop winning against the page behind them, and
   * this is the one setting whose whole purpose is legibility.
   */
  const opacity = () => {
    const wanted = Number(state.opacity);
    if (!Number.isFinite(wanted)) return 0.88;
    return Math.min(1, Math.max(0.6, wanted));
  };

  chrome.storage.onChanged.addListener(() => void loadState());

  // -------------------------------------------------------------------------------------
  // What is on the page
  // -------------------------------------------------------------------------------------

  /**
   * The text of a block, without anything this extension put there.
   *
   * Once an answer is drawn *inside* the text it answers, a plain `innerText` would read that
   * answer back as page content: it would be sent to the model as part of the page on the next
   * look, and -- worse -- it would change the text every time it was drawn, so the
   * unchanged-page check could never fire again and every mutation would cost a request. The
   * fast path is the same one `innerText` as before; the slow one is taken only for the few
   * blocks that actually carry one of our nodes.
   */
  function textOf(element) {
    if (!element.querySelector("[data-projectz-answer]")) return element.innerText || "";
    let text = "";
    const walk = (node) => {
      for (const child of node.childNodes) {
        if (child.nodeType === 3) {
          text += child.nodeValue;
          continue;
        }
        if (child.nodeType !== 1) continue;
        if (child.hasAttribute("data-projectz-answer") || SKIP.has(child.tagName)) continue;
        if (child.tagName === "BR") {
          text += "\n";
          continue;
        }
        walk(child);
        // A block-level child ends a line, which is what `innerText` would have done.
        if (!INLINE.has(child.tagName)) text += "\n";
      }
    };
    walk(element);
    return text;
  }

  /**
   * How many blocks one walk may *look at*, before the nearest ones are chosen to send.
   *
   * The caps below are a budget, and a budget spent from the top of the document spends itself on
   * the sidebar and the first screenful: on a chat whose newest message is the question at the
   * bottom, the walk stopped long before it arrived. So the walk reads further than it will send and
   * [`nearest`] spends the budget where the reader is. Bounded because `innerText` -- which is what
   * the walk is doing -- is the expensive part of it.
   */
  const SCAN_BLOCKS = 240;
  const SCAN_CHARS = MAX_FRAME_CHARS * 4;

  /**
   * The part of the page that is the *content*, rather than its chrome.
   *
   * A page's prose is usually inside `main`, `article` or `[role=main]`, while its navigation,
   * banners and footers are not. Reading the whole body and then filtering is how a sidebar
   * ends up spending the frame's budget -- and on a small model a menu item competes with the
   * actual question for attention. The fallback is `body`, so a page with no landmark still
   * works exactly as before; the threshold is what stops a tiny `<main>` with a cookie notice
   * in it from hiding the article.
   */
  function mainRoot() {
    for (const selector of ["main", "article", "[role=main]"]) {
      const element = document.querySelector(selector);
      if (element && (element.innerText || "").trim().length >= 200) return element;
    }
    return document.body;
  }

  /**
   * What the DOM already knows about a block, so the model does not have to infer it.
   *
   * A small model reading `print(x)` and `Preheat the oven` as flat text has to guess which is
   * code. The source knows, for free, and the prompt can say `(code) ...` -- which is the
   * difference between a `check` card and an `explain` card.
   */
  function hintOf(element) {
    const tag = element.tagName;
    if (tag === "PRE" || tag === "CODE") return "code";
    if (tag === "TABLE") return "table";
    if (tag === "IMG") return "image";
    if (/^H[1-6]$/.test(tag)) return "heading";
    return "";
  }

  /**
   * The text blocks on this page, in document order.
   *
   * **The whole page, not the visible part of it.** Reading only what is on screen made every count
   * and every number depend on where the reader had scrolled to -- a page looked at twice with the
   * same content came back as a different list, and a question just below the fold was never asked
   * about until it was scrolled to. A card for a block further down the page is simply drawn further
   * down the page.
   *
   * Bounded four ways -- per block, per look, by count, and by the app's own floor -- because this is
   * what crosses the wire on every look, and these are the caps that keep a long article from costing
   * a long article. The floor is the *app's* rather than a number of this file's own: it is the one
   * the user can change, and a block the app would refuse is a trip spent to be told so.
   */
  function collect() {
    const found = [];
    let total = 0;
    const floor = minChars();
    const cut = blockCut();

    const visit = (element) => {
      if (found.length >= SCAN_BLOCKS || total >= SCAN_CHARS) return;
      if (element.nodeType !== 1) return;
      if (SKIP.has(element.tagName)) return;
      // Chrome, not content: a navigation bar, a footer and an aside are the blocks that used
      // to fill the budget on a long article without ever holding a question.
      if (
        element.closest &&
        element.closest("nav, footer, aside, [role=navigation], [role=banner], [role=contentinfo]")
      ) {
        return;
      }
      // The app's own status page and anything the user is writing in: the first is not
      // content, and the second is someone's own words, which is exactly what the prompt
      // refuses to answer.
      if (element.isContentEditable) return;
      if (element.closest && element.closest(`#${HOST_ID}`)) return;
      // An answer of ours, in inline style, sitting where the page's own text would be.
      if (element.hasAttribute("data-projectz-answer")) return;
      if (element.getAttribute("aria-hidden") === "true") return;

      // A block with no box is not content, and `innerText` would hand back its *markup*: a
      // collapsed panel, an unopened tab or a `<template>` would otherwise be read as page text.
      // This is the one thing the viewport test was good for, kept without the viewport.
      const rect = element.getBoundingClientRect();
      if (rect.width === 0 || rect.height === 0) return;

      // An image with alt text is a block of its own: quiz screenshots, diagrams and "what does
      // this chart show" pages otherwise arrive as nothing at all. The alt text is not the
      // picture, but it is the one textual handle the page offers, and the app can still ask a
      // vision model about the image itself later.
      if (element.tagName === "IMG") {
        const alt = (element.getAttribute("alt") || "").replace(/\s+/g, " ").trim();
        if (alt.length >= floor) {
          found.push({ element, text: alt.slice(0, cut), rect, hint: "image" });
          total += alt.length;
        }
        return;
      }

      const hasBlockChild = Array.from(element.children).some(
        (child) => !INLINE.has(child.tagName)
      );
      if (hasBlockChild) {
        for (const child of Array.from(element.children)) visit(child);
        return;
      }

      const text = textOf(element).trim();
      if (text.length < floor) return;
      // Already answered, or already asked. Sending it again is the same question for the same
      // answer, and the answer is on screen already.
      if (answered.has(fnv(text))) return;
      found.push({ element, text: text.slice(0, cut), rect, hint: hintOf(element) });
      total += text.length;
    };

    const root = mainRoot();
    if (root) visit(root);
    return nearest(found, total);
  }

  /**
   * The blocks closest to where the reader is looking, in document order.
   *
   * **A budget has to be spent near the reader, or it is spent on the top of the page.** Collected
   * front to back, the caps went to the sidebar, the header and the opening paragraphs -- so on a
   * chat the newest message (the question at the bottom) was cut off before it was ever read, and a
   * long article's question halfway down never arrived. Nothing here excludes those blocks; they are
   * simply not the ones the budget buys first.
   *
   * The chosen blocks keep document order, because the model is reading a page: a list whose numbers
   * jump around the document is harder to reason about, and the grouping above assumes adjacency.
   */
  function nearest(found, scannedChars) {
    const limit = maxBlocks();
    if (found.length <= limit && scannedChars <= MAX_FRAME_CHARS) return found;

    const viewport = window.innerHeight;
    /** How far outside the screen a block was, in pixels. 0 is on it. */
    const distance = (entry) => {
      if (entry.rect.bottom < 0) return -entry.rect.bottom;
      if (entry.rect.top > viewport) return entry.rect.top - viewport;
      return 0;
    };

    const ranked = [...found].sort((left, right) => distance(left) - distance(right));
    const keep = new Set();
    let chars = 0;
    for (const entry of ranked) {
      if (keep.size >= limit) break;
      // A single unusually long block is skipped rather than ending the search: a shorter one
      // further down may still fit, and the budget is what decides.
      if (chars + entry.text.length > MAX_FRAME_CHARS) continue;
      keep.add(entry);
      chars += entry.text.length;
    }
    return found.filter((entry) => keep.has(entry));
  }

  /** Tags whose text belongs with the question above them rather than standing on its own. */
  const CONTEXT_TAGS = new Set(["PRE", "CODE", "TABLE", "UL", "OL", "DL", "FIGURE"]);

  /**
   * Folds a question and the block that belongs to it into one entry.
   *
   * A block is a leaf element, and the thing a question is *about* is usually its next sibling: the
   * code `What does this small program print?` asks about, the table a sum refers to, the options
   * under a quiz. Sent separately, the model has to jump between two numbered entries to answer
   * either -- and the code, which contains a digit beside a `*`, was ranked *above* the real
   * questions as arithmetic. Folded together, the question carries its own context, the model reads
   * one entry the way a person reads one card, and the card is drawn on the question rather than on
   * the sample underneath it.
   *
   * Only when the entry above genuinely asks something (a `?` or a colon at its end): an ordinary
   * paragraph followed by a list is two blocks, and merging those would send a page's prose as one
   * enormous candidate.
   */
  function group(collected) {
    const grouped = [];
    const cut = blockCut();
    for (const entry of collected) {
      const previous = grouped[grouped.length - 1];
      const asks = previous && /[?:]\s*$/.test(previous.text);
      if (asks && CONTEXT_TAGS.has(entry.element.tagName)) {
        // The question keeps its element and its rect: that is what the card is placed on. The
        // merged text is cut again, because two blocks that each fitted the budget are not one
        // block that does.
        previous.text = `${previous.text}\n${entry.text}`.slice(0, cut);
        // The context block's structure is now the merged block's: a question above a code
        // sample is a `check`, not a plain `answer`. A heading the question already carried is
        // the weaker hint and gives way.
        if (!previous.hint || previous.hint === "heading") previous.hint = hintOf(entry.element);
        continue;
      }
      grouped.push(entry);
    }
    return grouped;
  }

  /**
   * The page as a frame: numbered *and* named.
   *
   * The number is what the model answers with, because a small model handles `[3]` far better than a
   * hex string. The name is what the *card* is placed by, because a number is a position and this is
   * a live page: an answer that arrives after something appeared above it would otherwise be drawn
   * over whatever now sits at that number. A name the page can no longer find is dropped instead --
   * "not drawn" beats "drawn over the wrong words", which is the one failure worse than silence.
   */
  function frameOf() {
    const collected = group(collect());
    if (collected.length === 0) return null;
    blocks = new Map();
    keys = new Map();
    const frameBlocks = collected.map((entry, position) => {
      const index = position + 1;
      const key = fnv(entry.text).toString(36);
      blocks.set(index, entry.element);
      // First one wins: two paragraphs of the same text are one candidate to the app, so one
      // element is all a card about that text can ever need.
      if (!keys.has(key)) keys.set(key, entry.element);
      return {
        index,
        key,
        text: entry.text,
        tag: entry.element.tagName.toLowerCase(),
        // What the DOM knows about this block's structure. The app folds it into the prompt as
        // `(code) ...`, so a small model does not have to infer code from prose.
        hint: entry.hint || "",
        rect: {
          x: entry.rect.x,
          y: entry.rect.y,
          width: entry.rect.width,
          height: entry.rect.height,
        },
      };
    });
    return {
      id: `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`,
      url: location.href,
      title: document.title,
      blocks: frameBlocks,
    };
  }

  /** The element a card belongs to: by name where the app gave one, by number otherwise. */
  function blockFor(card) {
    return (card.key && keys.get(card.key)) || blocks.get(card.index);
  }

  // -------------------------------------------------------------------------------------
  // When to look
  // -------------------------------------------------------------------------------------

  const markActivity = () => {
    lastActivity = Date.now();
  };
  for (const type of ["keydown", "keyup", "pointerdown", "pointerup", "wheel", "scroll", "input", "focusin"]) {
    // Passive and capturing: this only watches, and it has to see scrolls inside nested
    // scrolling panels as well as the page itself.
    window.addEventListener(type, markActivity, { passive: true, capture: true });
  }

  /**
   * Looks, if looking is allowed *and* nothing is happening.
   *
   * The idle check is the difference between helping someone and interrupting them: while
   * a page is being scrolled through or typed into there is no reading at all, and a page
   * that goes still is read once, not once per tick.
   */
  function maybeLook(delay = 0) {
    if (!state.enabled || !state.auto || !allowedHere() || document.hidden) return;
    if (Date.now() < cooldownUntil) return;
    if (Date.now() - lastActivity < ACTIVE_MS) {
      // Someone is doing something. One timer, not one per event: the read happens when
      // they stop, not while they are working.
      if (!holdTimer) {
        holdTimer = window.setTimeout(() => {
          holdTimer = 0;
          maybeLook();
        }, ACTIVE_MS);
      }
      return;
    }
    if (reading) return;
    window.setTimeout(() => {
      if (Date.now() - lastActivity < ACTIVE_MS) return maybeLook();
      if (!state.enabled || !state.auto || !allowedHere() || document.hidden || reading) return;
      void look();
    }, delay);
  }

  /**
   * A fingerprint of a string: FNV-1a, a few instructions per character and no import.
   *
   * Used for three things at once, which is the point of it being one function: a block's *name* (so
   * an answer can be placed on what it was about rather than where that was), the record of what has
   * already been answered, and -- through [`hashOf`] -- whether a page has changed at all since the
   * last look. A collision costs one look that should not have happened, or one block considered
   * already answered; it can never place an answer wrongly, because placing is by exact text.
   */
  function fnv(text) {
    let hash = 2166136261;
    for (let index = 0; index < text.length; index += 1) {
      hash ^= text.charCodeAt(index);
      hash = Math.imul(hash, 16777619);
    }
    return hash >>> 0;
  }

  /**
   * A fingerprint of the text a look would send.
   *
   * The question it answers is "is this the same page?", so a newline is mixed between blocks: a
   * page whose two paragraphs became one is not the page that was sent a moment ago.
   */
  function hashOf(frame) {
    let hash = 2166136261;
    for (const block of frame.blocks) {
      hash ^= fnv(block.text);
      hash = Math.imul(hash, 16777619);
    }
    return hash >>> 0;
  }

  async function look() {
    const frame = frameOf();
    if (!frame) return;
    // **Nothing unchanged is sent twice.** The app remembers nothing between looks, on purpose,
    // so the only place that can know a page has not changed since the last one is here -- and
    // it is the cheapest saving there is, because what is not spent is the whole request. A
    // question written again is a different text and is answered again; the same page, still
    // sitting there, is not asked about twice. The popup's Read button ignores this.
    const hash = hashOf(frame);
    if (hash === lastSent) return;
    reading = Date.now();
    lastSent = hash;
    try {
      await chrome.runtime.sendMessage({ type: "page", frame });
    } catch {
      // The extension was reloaded under this tab, or the worker refused the message. Not
      // worth drawing anything about; the next look will either work or not.
      reading = 0;
      return;
    }
    // A reply that never comes -- the app quit mid-request -- must not leave this page
    // unable to look again for the rest of its life.
    window.setTimeout(() => {
      reading = 0;
    }, ANSWER_TIMEOUT_MS);
  }

  // A page that changes is worth another look. Watching the whole document and debouncing
  // is what makes a single-page app work: the question often arrives a second after the
  // navigation that appeared to end.
  const observer = new MutationObserver(() => {
    // Before anything else, and whatever the switch says: an answer that is already on screen
    // and has been removed by the page is put back, because this callback is the first thing to
    // run after that happens.
    reassertInline();
    if (!state.enabled || !state.auto || !allowedHere()) return;
    window.clearTimeout(settleTimer);
    settleTimer = window.setTimeout(() => maybeLook(), settleMs());
  });
  const startObserving = () => {
    if (document.body) {
      observer.observe(document.body, { childList: true, subtree: true, characterData: true });
    }
  };

  document.addEventListener("visibilitychange", () => {
    if (!document.hidden) {
      // Coming back to a tab is the commonest moment for a page to have changed under it.
      window.clearTimeout(settleTimer);
      settleTimer = window.setTimeout(() => maybeLook(settleMs()), 200);
    }
  });

  // -------------------------------------------------------------------------------------
  // Drawing
  // -------------------------------------------------------------------------------------

  const STYLES = `
    /*
     * The host is the layer and nothing else.
     *
     * **Zero-sized, fixed, and not a hit target.** A host that covers the viewport is an
     * invisible sheet over the page, and one property going missing makes every link, button
     * and text field under it stop responding -- which is exactly what happened:
     * "all: initial" resolved pointer-events to auto, and the declaration meant to undo that
     * did not survive alongside it. A zero-sized box cannot swallow anything even if every
     * style on it is lost, so the failure mode of this file is "no card appears" rather than
     * "the page stops working".
     *
     * The layer inside is fixed to the viewport and carries the stacking order; it passes
     * pointer events through, and only the cards take them back.
     */
    :host {
      position: fixed; top: 0; left: 0; width: 0; height: 0;
      z-index: 2147483647; pointer-events: none; overflow: visible;
    }
    .layer { position: fixed; inset: 0; pointer-events: none; }
    .card {
      position: absolute; top: 0; left: 0; width: ${CARD_WIDTH}px;
      max-height: ${CARD_MAX_HEIGHT}px; display: flex; flex-direction: column;
      pointer-events: auto; cursor: default;
      border-radius: 11px; border: 1px solid var(--line);
      /* See-through enough that the paragraph underneath is still the thing being read, opaque
         enough that the answer's own words are. The number is a setting; the floor is
         legibility, which is why the field starts at 0.6 and the default sits at 0.88. Mixed
         into the background rather than set as CSS opacity, which would fade the answer's text
         with it and make a translucent card unreadable over a busy page. */
      background: color-mix(in srgb, var(--panel) calc(var(--projectz-solid, 0.88) * 100%), transparent);
      backdrop-filter: blur(6px);
      color: var(--ink);
      box-shadow: 0 8px 26px rgba(0, 0, 0, 0.18);
      font: 13px/1.5 ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif;
      overflow: hidden; transition: opacity 140ms ease;
    }
    .card[data-fading="true"] { opacity: 0; }
    .head {
      display: flex; align-items: baseline; gap: 8px;
      padding: 8px 10px 7px; border-bottom: 1px solid var(--line);
    }
    .mark { font-size: 10.5px; letter-spacing: 0.04em; color: var(--quiet); }
    .meta { margin-left: auto; font-size: 10.5px; color: var(--quiet); }
    .body { padding: 9px 11px 10px; overflow-y: auto; }
    .answer { margin: 0; white-space: pre-wrap; color: var(--ink); }
    .silent { margin: 0; color: var(--muted); }
    .reason { margin: 6px 0 0; font-size: 11px; color: var(--quiet); }
    .foot {
      display: flex; align-items: center; gap: 2px;
      border-top: 1px solid var(--line); padding: 4px 6px;
    }
    button {
      all: unset; cursor: pointer; border-radius: 6px;
      padding: 4px 7px; font-size: 11px; color: var(--muted);
    }
    button:hover { background: var(--raise); color: var(--ink); }
    button:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
    .spacer { flex: 1; }
    @media (prefers-color-scheme: dark) {
      .card { --panel: #1b1b1f; --ink: #f1f1f3; --muted: #a0a0aa; --quiet: #7c7c87;
              --line: #2e2e36; --raise: #26262d; --accent: #dda45c; }
    }
    @media (prefers-color-scheme: light) {
      .card { --panel: #ffffff; --ink: #17171a; --muted: #5f5f6a; --quiet: #8a8a94;
              --line: #e5e5e0; --raise: #f2f2ee; --accent: #a8621a; }
    }
  `;

  function hostLayer() {
    if (layer) return layer;
    // **A host left by an earlier run of this script is removed before ours exists.**
    // Reloading the extension does not clean up the DOM its content script injected: the old
    // instance dies, its element stays, and a full-viewport version of it blocks the page for
    // as long as the tab is open. There is exactly one host after this line.
    for (const stale of document.querySelectorAll(`#${HOST_ID}`)) stale.remove();
    const hostElement = document.createElement("div");
    hostElement.id = HOST_ID;
    // No inline styles. The host is positioned by the `:host` rule in its own shadow
    // stylesheet, which is the one place that can style it, and everything about it is chosen
    // so that losing the stylesheet costs the card rather than the page's input.
    document.documentElement.appendChild(hostElement);
    const shadow = hostElement.attachShadow({ mode: "open" });
    const style = document.createElement("style");
    style.textContent = STYLES;
    const container = document.createElement("div");
    container.className = "layer";
    shadow.append(style, container);
    layer = container;
    return layer;
  }

  /**
   * Removes the host entirely, and forgets the layer with it.
   *
   * Called when the last card goes. A page the user is not being helped on must have no
   * injected element in it at all -- not even an empty one -- so there is nothing that can
   * stand between a click and the page.
   *
   * It also sweeps away any aside left by an *earlier* run of this script, which is the one
   * thing a reloaded extension cannot do for itself: the old instance's closure is gone, so
   * nothing it drew belongs to anything that is still running, and a stale `// 56` beside a
   * paragraph could not be clicked away by anybody. Called at startup for that reason.
   */
  function teardownLayer() {
    const host = document.getElementById(HOST_ID);
    if (host) host.remove();
    for (const stale of document.querySelectorAll("[data-projectz-answer]")) stale.remove();
    layer = null;
  }

  /**
   * Where a card goes for a block.
   *
   * Over the block's own top right when the block is wide enough for it -- the way the
   * app's own preview draws it, and the way a card belongs to the text it answers -- and
   * otherwise beside it, or above or below it, according to what the user asked for. The
   * horizontal clamp is what keeps a card for a narrow column from hanging off the side of
   * the window, where it could not be read at all.
   */
  function placeFor(rect, height) {
    const gap = 8;
    const maxLeft = Math.max(8, window.innerWidth - CARD_WIDTH - 8);
    const clamp = (left) => Math.min(Math.max(8, left), maxLeft);
    const roomInside = rect.width >= CARD_WIDTH + 72;
    if (roomInside) {
      return { left: clamp(rect.right - CARD_WIDTH - 8), top: rect.top + 6 };
    }
    const roomRight = rect.right + gap + CARD_WIDTH <= window.innerWidth - 8;
    if (roomRight && state.card !== "below") {
      return { left: clamp(rect.right + gap), top: rect.top };
    }
    if (state.card === "above") {
      return { left: clamp(rect.left), top: Math.max(8, rect.top - height - gap) };
    }
    // `auto` and `below` both land here: under the block's own left edge, which is where a
    // card goes when there is nothing to its right.
    return { left: clamp(rect.left), top: rect.bottom + gap };
  }

  function position(card) {
    const element = blockFor(card);
    if (!element || !element.isConnected) {
      // The block is gone from the page: the card has nothing to point at, and an overlay
      // floating over whatever replaced it would be pointing at the wrong thing.
      remove(card.index);
      return;
    }
    // An answer drawn as part of the text is moved by the text; there is nothing to place.
    if (card.inline) return;
    const rect = element.getBoundingClientRect();
    const height = Math.min(card.element.offsetHeight || 120, CARD_MAX_HEIGHT);
    const { left, top } = placeFor(rect, height);
    card.element.style.transform = `translate(${Math.round(left)}px, ${Math.round(top)}px)`;
  }

  /**
   * Puts back an inline answer the page has thrown away.
   *
   * A framework re-rendering the paragraph it belongs to removes our node along with everything
   * else in there, because from the page's point of view it is just a span that was never
   * supposed to exist. The card style never needs this -- its layer lives outside the page's own
   * tree -- but an answer appended to the text does, and without it one re-render would take the
   * answer off the screen while the app went on believing it was there.
   *
   * Runs on every mutation, over the handful of answers on screen, and costs a property read
   * each when there are none -- which is the usual case.
   */
  function reassertInline() {
    for (const card of cards.values()) {
      if (!card.inline || card.element.isConnected) continue;
      const target = blockFor(card);
      if (!target || !target.isConnected) {
        remove(card.index);
        continue;
      }
      // Not beside ourselves: a look in between may have drawn this block again.
      if (!target.querySelector("[data-projectz-answer]")) {
        target.insertAdjacentElement("afterend", card.element);
      }
    }
  }

  function repositionAll() {
    raf = 0;
    for (const card of cards.values()) position(card);
    if (cards.size > 0) scheduleReposition();
  }
  function scheduleReposition() {
    if (raf) return;
    raf = requestAnimationFrame(repositionAll);
  }
  // Capturing, because the paragraph a card points at is usually inside a scrolling panel
  // rather than the window itself.
  window.addEventListener("scroll", scheduleReposition, { passive: true, capture: true });
  window.addEventListener("resize", scheduleReposition, { passive: true });

  function remove(index) {
    const card = cards.get(index);
    if (!card) return;
    card.element.remove();
    cards.delete(index);
    // Nothing left to draw means nothing left to stand over the page: the host goes with the
    // last card.
    if (cards.size === 0) teardownLayer();
  }

  function clear() {
    for (const index of Array.from(cards.keys())) remove(index);
  }

  function dismissAllNow() {
    clear();
  }

  function drawOne(entry) {
    // A style change takes what is already on screen with it: a card and an aside beside the
    // same paragraph would be one answer drawn twice.
    const style = cardStyle();
    if (drawnWith !== style) {
      if (drawnWith) clear();
      drawnWith = style;
    }
    // **Answered, and never asked about again.** Marked as the card is drawn rather than when it is
    // dismissed: the cost was already paid, and asking the same question again because the reader
    // clicked the answer away would buy the same answer back for a second time.
    if (entry.key) answered.add(entry.key);
    if (style === "inline") {
      drawInline(entry);
      return;
    }
    const root = hostLayer();
    const existing = cards.get(entry.index);
    // Replacing rather than stacking: a second card for the same paragraph would sit on
    // top of the first and hide it.
    if (existing) existing.element.remove();

    const card = document.createElement("article");
    card.className = "card";
    card.setAttribute("role", "note");
    // How solid the card is, as a custom property the stylesheet reads: the setting is the
    // number, and what it means for the background is the stylesheet's business.
    card.style.setProperty("--projectz-solid", String(opacity()));

    const head = document.createElement("div");
    head.className = "head";
    head.append(
      Object.assign(document.createElement("span"), { className: "mark", textContent: "Assist" })
    );
    const meta = document.createElement("span");
    meta.className = "meta";
    // The kind of help is the card's title now; the shape is only a fallback for an app built
    // before kinds existed. `answer` is the word a reader already expects.
    meta.textContent = [entry.kind || entry.shape, entry.subject !== "other" ? entry.subject : ""]
      .filter(Boolean)
      .join(" · ");
    head.append(meta);

    const body = document.createElement("div");
    body.className = "body";
    const answer = document.createElement("p");
    answer.className = "answer";
    answer.textContent = entry.answer;
    body.append(answer);
    if (typeof entry.confidence === "number" && entry.confidence > 0) {
      const confidence = document.createElement("p");
      confidence.className = "reason";
      confidence.textContent = `${Math.round(entry.confidence * 100)}% sure`;
      body.append(confidence);
    }

    const foot = document.createElement("div");
    foot.className = "foot";
    const copy = document.createElement("button");
    copy.type = "button";
    copy.textContent = "Copy";
    copy.addEventListener("click", (event) => {
      event.stopPropagation();
      void navigator.clipboard.writeText(entry.answer).then(() => {
        copy.textContent = "Copied";
        window.setTimeout(() => {
          copy.textContent = "Copy";
        }, 1400);
      });
    });
    foot.append(copy);

    card.append(head, body, foot);

    // **A click anywhere on the card makes it go away.** It is an answer to something
    // nobody asked for, so the least it can do is leave without being aimed at -- and the
    // two buttons above stop the event before it reaches here.
    card.addEventListener("click", () => remove(entry.index));
    // The page underneath must not see the press either: a card over a link that selected
    // text or followed it would be worse than no card.
    for (const type of ["pointerdown", "mousedown", "mouseup", "dblclick"]) {
      card.addEventListener(type, (event) => event.stopPropagation());
    }

    root.append(card);
    cards.set(entry.index, {
      index: entry.index,
      key: entry.key || "",
      element: card,
      inline: false,
    });
    position(cards.get(entry.index));

    const hideAfter = Number(state.hideAfter) || 0;
    if (hideAfter > 0) {
      window.setTimeout(() => {
        const current = cards.get(entry.index);
        if (current !== card) return;
        card.dataset.fading = "true";
        window.setTimeout(() => remove(entry.index), 200);
      }, hideAfter * 1000);
    }
  }

  /**
   * An answer drawn as part of the text it answers.
   *
   * A sibling *after* the block rather than characters written into it, and that is the whole of
   * the difference from a card. The node is still the page's, so the page's own scripts can
   * remove it -- which is why [`reassertInline`] exists -- but writing *inside* the paragraph
   * would put our words in the page's text flow, where the site's own code could read them, a
   * screen reader would announce them as part of the article, and a framework re-render would
   * splice them into the middle of a sentence. After the block is the readable half of that idea
   * with none of the damage.
   *
   * Styled inline rather than by class: this node lives in the page's own DOM, where a site's
   * stylesheet reaches it, and inline declarations beat every selector that is not `!important`.
   * The colour is mixed out of `currentColor` -- the colour the words beside it are written in --
   * so the aside looks like an aside on a dark site and on a light one, without a second palette
   * to keep in step with the page's.
   */
  function drawInline(entry) {
    const target = blockFor(entry);
    if (!target || !target.isConnected) return;
    const existing = cards.get(entry.index);
    if (existing) existing.element.remove();

    const note = document.createElement("span");
    // The attribute is what marks the node as ours -- what is collected skips it, and it is what
    // a re-render is checked against. Its value is the shape, which is only ever read by someone
    // inspecting the page.
    note.setAttribute("data-projectz-answer", entry.kind || entry.shape || "answer");
    note.setAttribute("role", "note");
    note.textContent = `// ${entry.answer}`;
    // The card's metadata has nowhere to be drawn here, so it goes where a hover finds it: the
    // shape, the subject, how sure, and what a click does.
    note.title = [
      "ProjectZ Assist",
      entry.kind || entry.shape,
      entry.subject && entry.subject !== "other" ? entry.subject : "",
      typeof entry.confidence === "number" && entry.confidence > 0
        ? `${Math.round(entry.confidence * 100)}% sure`
        : "",
      "click to dismiss",
    ]
      .filter(Boolean)
      .join(" · ");
    note.style.cssText = [
      "margin-left: 0.6em",
      "cursor: pointer",
      "font-style: italic",
      "color: color-mix(in srgb, currentColor 60%, transparent)",
    ].join(";");

    note.addEventListener("click", (event) => {
      // Both halves matter: the click dismisses the answer and must not also follow whatever
      // the paragraph belongs to, which is usually a link on a page like this.
      event.preventDefault();
      event.stopPropagation();
      remove(entry.index);
    });
    for (const type of ["pointerdown", "mousedown", "mouseup", "dblclick"]) {
      note.addEventListener(type, (event) => event.stopPropagation());
    }

    target.insertAdjacentElement("afterend", note);
    cards.set(entry.index, {
      index: entry.index,
      key: entry.key || "",
      element: note,
      inline: true,
    });
  }

  // -------------------------------------------------------------------------------------
  // Messages from the worker
  // -------------------------------------------------------------------------------------

  chrome.runtime.onMessage.addListener((message) => {
    if (!message || typeof message.type !== "string") return;

    if (message.type === "cards") {
      reading = 0;
      if (!state.enabled || !allowedHere()) return;
      // A page that cannot be answered right now is left alone for a while rather than
      // re-sent on every mutation. The app remembers what it *decided*; these are the cases
      // where it decided nothing -- no model chosen, the hour's budget spent -- so the
      // waiting has to happen on this side.
      if (message.stage === "no_model" || message.stage === "over_budget") {
        cooldownUntil = Date.now() + COOLDOWN_MS;
        // Forgotten as well as cooled down: nothing was answered, so the same text is worth
        // sending again once an hour has passed or a model has been chosen -- and a page that
        // gave up its fingerprint here would otherwise stay silent for as long as it was open.
        lastSent = -1;
        return;
      }
      if (message.stage === "failed") {
        cooldownUntil = Date.now() + FAILED_COOLDOWN_MS;
        lastSent = -1;
        return;
      }
      if (message.display === false && message.cards && message.cards.length > 0) {
        // Dry run: the app said nothing is to be drawn, so the answer is dropped rather than
        // shown. Logged so "why is nothing happening" has an answer for the one mode that is
        // meant to be silent.
        console.info(
          `[assist] dry run: ${message.cards.length} answer(s) not drawn (stage ${message.stage})`
        );
        return;
      }
      for (const entry of message.cards || []) {
        // **Placed by name, not by number.** A page that moved under an answer that was in flight --
        // a list re-rendered, a block inserted above -- leaves the number pointing at different
        // words, and an answer drawn there is worse than no answer. A name this page can no longer
        // find is dropped, and an app too old to send one falls back to the number it always used.
        if (entry.key && !keys.has(entry.key)) continue;
        drawOne(entry);
      }
      return;
    }

    if (message.type === "read") {
      // The popup's button: look now, whether or not the page has settled, and whatever the
      // cooldown said -- the user asking is the one thing that outranks the waiting. It clears the
      // fingerprint too, so a page whose answer was dismissed or never came is looked at again.
      //
      // It does *not* clear what has already been answered: the answers are on screen, and Read
      // means "look at what is new", not "buy those answers a second time".
      cooldownUntil = 0;
      lastSent = -1;
      void loadState().then(() => look());
      return;
    }

    if (message.type === "state") {
      if (!message.enabled) dismissAllNow();
      return;
    }

    if (message.type === "offline") {
      reading = 0;
    }
  });

  // -------------------------------------------------------------------------------------
  // What a page gets when it opens
  // -------------------------------------------------------------------------------------

  /**
   * The app's own setup page carries the token, so the extension pairs itself.
   *
   * This is the whole reason the app serves a page: nobody should have to find a token,
   * copy it and paste it into a popup. The token is read out of the page it was served to
   * -- the same origin the extension is allowed to run in -- and handed to the worker,
   * which is the only part of this that can open the socket.
   */
  function pairFromSetupPage() {
    const onApp = (location.hostname === "127.0.0.1" || location.hostname === "localhost") && location.pathname === "/";
    if (!onApp) return;
    const read = () => {
      const element = document.getElementById("token");
      const token = element && element.textContent ? element.textContent.trim() : "";
      // A token that is still the page's placeholder is not a token.
      if (token.length >= 32) {
        chrome.runtime.sendMessage({ type: "pair", token, port: Number(location.port) || 43128 }).catch(() => undefined);
      }
    };
    // Twice, because the page fills the token in from its own fetch and that can land
    // after this runs.
    window.setTimeout(read, 500);
    window.setTimeout(read, 2000);
  }

  void loadState().then(() => {
    // A layer from a previous instance of this script must not survive into this one: its
    // closure is gone, so nothing can close it, and an older version of it covered the page
    // for as long as the tab stayed open.
    teardownLayer();
    startObserving();
    pairFromSetupPage();
    // The first look is one settle period in, so a page that is still rendering when this
    // runs is not read in half.
    window.setTimeout(() => maybeLook(settleMs()), 400);
  });
})();
