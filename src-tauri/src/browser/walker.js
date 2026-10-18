//! The tree the model reads.
//!
//! # Why a tree and not a screenshot
//!
//! Everything here is a *rule about elements*, applied to whatever page is
//! loaded. There is no selector for a particular site, no branch for a particular
//! layout, and nothing that has to be updated when a page is redesigned -- the
//! site-specific knowledge lives in the model, per request, and is gone when the
//! run ends. A driver that knows one site is a driver that works on exactly one.
//!
//! # What is emitted and what is not
//!
//! A line for every element a person could *act on*, and for every element that
//! carries text of its own. Everything between them -- the layout wrappers, the
//! flex containers, the invisible-until-hovered menus -- is walked through and
//! produces nothing. That is what keeps the tree short enough to read on a real
//! page: markup is mostly structure, and structure is not information.
//!
//! # Why the numbers are stable
//!
//! An element's number is assigned once and kept, in a `WeakMap` keyed by the
//! element itself, so `[7]` means the same thing on the next call as it did on
//! the last. Numbers are never reused. A page that navigates is a new document
//! and its numbers start again, which is correct -- every element really is a
//! different one.
//!
//! A weak reference rather than a strong one in the reverse map, because a page
//! that re-renders throws away most of its elements, and holding them would be a
//! leak the size of everything the page has ever shown.

(function () {
  'use strict';

  // What a person can act on. Roles rather than only tags, because a site that
  // builds its own button out of a `div` is a button, and a driver that only
  // recognises `button` is a driver that cannot press it.
  var INTERACTIVE = [
    'a[href]',
    'button',
    'input',
    'select',
    'textarea',
    'summary',
    'label',
    'audio[controls]',
    'video[controls]',
    '[contenteditable="true"]',
    '[role="button"]',
    '[role="link"]',
    '[role="checkbox"]',
    '[role="radio"]',
    '[role="tab"]',
    '[role="menuitem"]',
    '[role="menuitemcheckbox"]',
    '[role="menuitemradio"]',
    '[role="option"]',
    '[role="switch"]',
    '[role="combobox"]',
    '[role="textbox"]',
    '[role="searchbox"]',
    '[role="slider"]',
    '[role="spinbutton"]',
    '[onclick]',
    '[tabindex]:not([tabindex="-1"])'
  ].join(',');

  // Read for every element, whichever site it is. Chosen for what changes a
  // decision: what kind of control this is, what it is called, whether it is
  // already filled in, and where it goes. Deliberately short -- an attribute
  // nobody acts on is prompt space spent on nothing.
  var ATTRIBUTES = [
    'type',
    'name',
    'id',
    'placeholder',
    'value',
    'href',
    'src',
    'title',
    'alt',
    'aria-label',
    'aria-expanded',
    'aria-checked',
    'aria-selected',
    'role',
    'checked',
    'disabled',
    'selected',
    'multiple',
    'maxlength',
    'autocomplete',
    'for'
  ];

  // Subtrees that hold no readable content. Skipped whole rather than filtered
  // line by line, because on a real page they are most of the node count.
  var SKIP = {
    SCRIPT: 1,
    STYLE: 1,
    NOSCRIPT: 1,
    TEMPLATE: 1,
    HEAD: 1,
    META: 1,
    LINK: 1,
    TITLE: 1,
    SVG: 1,
    PATH: 1
  };

  var MAX_LINES = 800;
  var MAX_VISITED = 6000;
  /// A guard on a selector that matches half a page, so one call cannot walk the
  /// whole document several times over.
  var MAX_MATCHES = 400;
  var MAX_DEPTH = 14;
  var MAX_TEXT = 160;
  var MAX_ATTRIBUTE = 80;
  var INDENT = '  ';

  function state() {
    return (window.__pzState =
      window.__pzState || { next: 0, ids: new WeakMap(), refs: new Map() });
  }

  function collapse(value) {
    return String(value == null ? '' : value)
      .replace(/\s+/g, ' ')
      .trim();
  }

  function shorten(text, limit) {
    return text.length > limit ? text.slice(0, limit - 1) + '…' : text;
  }

  /// The number for this element, assigning one the first time it is seen.
  ///
  /// Exposed as well as used, because `find` reports the same numbers a snapshot
  /// does -- a search narrows the tree rather than being a second numbering that
  /// has to be kept in step with it.
  function indexOf(element) {
    var known = state();
    var index = known.ids.get(element);
    if (index === undefined) {
      index = known.next++;
      known.ids.set(element, index);
    }
    // Re-registered whenever the reference has been cleared. A `WeakRef` goes
    // dead when its element is collected, and the index would then point at
    // nothing -- re-adding it here means a re-rendered element still resolves.
    var reference = known.refs.get(index);
    if (!reference || !reference.deref()) {
      known.refs.set(index, new WeakRef(element));
    }
    return index;
  }

  window.__pzIndex = indexOf;

  /// The element a number names, or why it no longer names anything.
  ///
  /// The error is written for the model, because it is the model that has to act
  /// on it: a page that re-rendered needs a new snapshot, and saying so is
  /// cheaper than a click that lands on whatever took the old element's place.
  window.__pzElement = function (index) {
    var known = window.__pzState;
    var reference = known && known.refs.get(index);
    var element = reference && reference.deref();
    if (!element || !element.isConnected) {
      throw new Error(
        'element [' + index + '] is no longer on the page. Take a snapshot and use its numbers.'
      );
    }
    return element;
  };

  function isVisible(element) {
    var view = element.ownerDocument && element.ownerDocument.defaultView;
    if (!view) return true;
    var style = view.getComputedStyle(element);
    if (!style) return true;
    if (
      style.display === 'none' ||
      style.visibility === 'hidden' ||
      style.visibility === 'collapse' ||
      style.opacity === '0'
    ) {
      return false;
    }
    // A box with no area cannot be clicked and cannot be seen. This is also what
    // excludes an `<option>`, whose text belongs to the `<select>` it lives in.
    var rect = element.getBoundingClientRect();
    return rect.width >= 1 || rect.height >= 1;
  }

  /// The element's own text, not its children's.
  ///
  /// Direct children only, so `[3]<p>Hello` followed by a child's `world` reads
  /// as the two things a reader sees rather than the same words twice.
  function inlineText(element) {
    var text = '';
    for (var node = element.firstChild; node; node = node.nextSibling) {
      if (node.nodeType === 3) text += ' ' + node.nodeValue;
    }
    return shorten(collapse(text), MAX_TEXT);
  }

  /// One line for one element: its number, its tag, what it is set to.
  ///
  /// The number is part of the line and not something the caller adds, because
  /// this is the only place that knows whether the element has one -- a plain
  /// paragraph has nothing to address, and printing a number for it would offer
  /// the model a handle on something it cannot act on.
  function describe(element, index) {
    var parts = [];
    for (var i = 0; i < ATTRIBUTES.length; i++) {
      var name = ATTRIBUTES[i];
      if (!element.hasAttribute(name)) continue;
      if (name === 'value' && element.type === 'password') {
        parts.push('value="••••"');
        continue;
      }
      var value = shorten(collapse(element.getAttribute(name)), MAX_ATTRIBUTE);
      // A boolean attribute says everything by being present.
      parts.push(value ? name + '="' + value + '"' : name);
    }

    // Two facts live on the element rather than in its markup, and they are the
    // ones a decision turns on: whether a box is ticked, and which option a menu
    // is showing. Read from the live element, because the attribute is the state
    // it *started* in.
    if (element.type === 'checkbox' || element.type === 'radio') {
      parts.push(element.checked ? 'checked' : 'unchecked');
    }
    var tag = element.tagName.toLowerCase();
    if (tag === 'select') {
      var chosen = element.selectedOptions && element.selectedOptions[0];
      if (chosen) parts.push('selected="' + shorten(collapse(chosen.textContent), MAX_ATTRIBUTE) + '"');
    }

    // The `>` closes the open tag, so what follows reads as the element's text
    // rather than as one more attribute.
    return (
      (index === null ? '' : '[' + index + ']') +
      '<' + tag +
      (parts.length ? ' ' + parts.join(' ') : '') +
      '>'
    );
  }

  function indent(depth) {
    var levels = Math.min(depth, MAX_DEPTH);
    var text = '';
    for (var i = 0; i < levels; i++) text += INDENT;
    return text;
  }

  function walk(node, depth, out) {
    for (var child = node.firstChild; child; child = child.nextSibling) {
      if (out.lines.length >= MAX_LINES || out.visited >= MAX_VISITED) return;
      // Text belongs to its parent's line, so a bare text node has nothing of its
      // own to say.
      if (child.nodeType !== 1) continue;
      if (SKIP[child.tagName]) continue;
      out.visited++;

      // A same-origin frame is part of the page: its elements join the same
      // numbering, and a click into it is resolved to parent coordinates by the
      // prelude. A cross-origin frame has no readable document, and is shown as
      // the one row it honestly is.
      var frame = null;
      if (child.tagName === 'IFRAME' || child.tagName === 'FRAME') {
        try {
          frame = child.contentDocument;
        } catch (error) {
          frame = null;
        }
      }
      var visible = frame ? true : isVisible(child);
      if (!visible) continue;

      var interactive = false;
      try {
        interactive = child.matches(INTERACTIVE);
      } catch (error) {
        interactive = false;
      }
      var text = inlineText(child);
      var index = interactive ? indexOf(child) : null;

      // In interactive mode only the actionable lines are emitted; the rest of
      // the page is still walked, because a text row's children may be the
      // buttons. Full mode adds the text, which is what a page *says*.
      var emit = out.mode === 'interactive' ? index !== null : index !== null || text !== '';
      if (emit) {
        out.lines.push(
          indent(depth) + (index === null ? text : describe(child, index) + (text ? ' ' + text : ''))
        );
      }

      // A shadow root is part of the page and is invisible to `querySelector`,
      // so an app built from components would otherwise return a tree with its
      // content missing -- which reads as "the page is empty", and is worse than a
      // long tree. Descended into *alongside* the light children, because the light
      // children are what a slot shows: an assigned node lives in the light tree
      // and appears in the shadow tree only as a slot with no children of its own,
      // so nothing is walked twice.
      if (child.shadowRoot && depth + 1 < MAX_DEPTH) walk(child.shadowRoot, depth + 1, out);

      if (depth + 1 < MAX_DEPTH) walk(frame || child, depth + 1, out);
    }
  }

  /// One pass over a subtree, emitting the lines it produces.
  function walkOnce(mode, root) {
    var out = { lines: [], visited: 0, mode: mode };
    walk(root, 0, out);
    return out;
  }

  /// The page as text, numbered.
  ///
  /// `mode` is `"full"` (the default) or `"interactive"` for just the elements
  /// that can be acted on. `"auto"` is accepted and means the full tree: it used
  /// to fall back to the controls on a long page, which was a mistake worth
  /// stating -- the text it dropped is precisely what a reader came for, and a
  /// model cannot search what is not there while it can ignore what is. The cap
  /// below still bounds the result, and a caller that knows which part it wants
  /// has `root` to say so.
  ///
  /// `root` scopes the walk to one part of the page, which is the honest way to
  /// make a long page short: the model knows which part it needs.
  window.__pzWalk = function (mode, root) {
    var start = root || document.body || document.documentElement;
    var out = walkOnce(mode === 'interactive' ? 'interactive' : 'full', start);

    var text = out.lines.join('\n');
    if (out.lines.length >= MAX_LINES) {
      text +=
        '\n… this page is longer than one tree. Search it with findText(text), narrow it with snapshot(selector), or read one part with text(selector).';
    }
    return text;
  };

  /// Every element matching a selector, shadow roots included.
  ///
  /// `querySelectorAll` does not see into a shadow root, so a selector naming
  /// something inside one matches nothing -- and from the outside that is
  /// indistinguishable from a typo in the selector. This descends through open
  /// roots, which is the same reach the tree walk has, so `snapshot('.thing')` and
  /// a `.thing` in the tree agree about what exists.
  function queryAll(selector, root) {
    var found = [];
    var collect = function (node) {
      try {
        var matched = node.querySelectorAll(selector);
        for (var i = 0; i < matched.length; i++) found.push(matched[i]);
      } catch (error) {
        // A selector the browser will not parse is the caller's mistake and is
        // reported as "nothing matched" by whoever asked; nothing here can say
        // anything better, and throwing would take the whole script down.
        return;
      }
      if (found.length > MAX_MATCHES) return;
      var every = node.querySelectorAll('*');
      for (var j = 0; j < every.length; j++) {
        if (every[j].shadowRoot) collect(every[j].shadowRoot);
      }
    };
    collect(root || document);
    return found;
  }

  window.__pzQueryAll = queryAll;
  window.__pzQuery = function (selector, root) {
    return queryAll(selector, root)[0] || null;
  };

  /// Whether a person could act on this element.
  ///
  /// Exposed because the search has to treat these differently: an actionable
  /// element is labelled by everything inside it, while a wrapper is labelled
  /// only by its own text. Without the distinction, a search returns every
  /// ancestor above a match as well as the match.
  window.__pzInteractive = function (node) {
    try {
      return node.matches(INTERACTIVE);
    } catch (error) {
      return false;
    }
  };

  /// Elements whose text starts a new line.
  ///
  /// The only thing `innerText` does that a walk does not, and the reason a
  /// collected page still reads like a page instead of one run-on line.
  var BLOCK = {
    ADDRESS: 1,
    ARTICLE: 1,
    ASIDE: 1,
    BLOCKQUOTE: 1,
    BR: 1,
    DD: 1,
    DIV: 1,
    DL: 1,
    DT: 1,
    FIELDSET: 1,
    FIGCAPTION: 1,
    FIGURE: 1,
    FOOTER: 1,
    FORM: 1,
    H1: 1,
    H2: 1,
    H3: 1,
    H4: 1,
    H5: 1,
    H6: 1,
    HEADER: 1,
    HR: 1,
    LI: 1,
    MAIN: 1,
    NAV: 1,
    OL: 1,
    P: 1,
    PRE: 1,
    SECTION: 1,
    TABLE: 1,
    TBODY: 1,
    TD: 1,
    TFOOT: 1,
    TH: 1,
    THEAD: 1,
    TR: 1,
    UL: 1
  };

  function collectText(node, out) {
    if (node.nodeType === 3) {
      out.push(node.nodeValue);
      return;
    }
    // A fragment -- which a shadow root is -- and a document are not elements, so
    // they have no tag to inspect and no text of their own; their children carry
    // both. Returning here instead of descending is what made a shadow root read
    // as empty text while its elements were in the tree.
    if (node.nodeType === 9 || node.nodeType === 11) {
      for (var inside = node.firstChild; inside; inside = inside.nextSibling) {
        collectText(inside, out);
      }
      return;
    }
    if (node.nodeType !== 1) return;
    if (SKIP[node.tagName]) return;
    // Hidden content is not part of the page a reader sees, and a template's
    // contents are not part of it at all. `aria-hidden` is authored for exactly
    // this purpose, so it is believed.
    if (node.hidden) return;
    if (node.getAttribute && node.getAttribute('aria-hidden') === 'true') return;

    // A same-origin frame is part of the page, and its text is part of the page's
    // text. The tree already walks into one; a reader that did not would call a
    // message body empty while its elements were right there -- and when the frame
    // sits inside a shadow root, `querySelectorAll('iframe')` cannot even see that
    // it exists, so this is the only path to the text.
    if (node.tagName === 'IFRAME' || node.tagName === 'FRAME') {
      var frame = null;
      try {
        frame = node.contentDocument;
      } catch (error) {
        frame = null;
      }
      // A cross-origin frame has no readable document, which is why the tree shows
      // it as the one row it is rather than pretending there is content.
      if (frame) collectText(frame.body || frame.documentElement, out);
      return;
    }

    var block = BLOCK[node.tagName];
    if (block) out.push('\n');
    // The reason this exists: `innerText` stops at a shadow boundary, so on a
    // component-built page the readable text was the chrome and the message body
    // was missing entirely -- while every element was in the tree.
    if (node.shadowRoot) collectText(node.shadowRoot, out);
    for (var child = node.firstChild; child; child = child.nextSibling) {
      collectText(child, out);
    }
    if (block) out.push('\n');
  }

  /// The page's text, as a reader sees it, shadow roots included.
  ///
  /// Used for both `text` and `findText`, so the two can never disagree about what
  /// a page says -- which is what a search that misses text the reader can see
  /// would be.
  window.__pzText = function (root) {
    var out = [];
    collectText(root || document.body || document.documentElement, out);
    return out
      .join('')
      .split('\n')
      .map(function (line) {
        return line.replace(/\s+/g, ' ').trim();
      })
      .filter(function (line) {
        return line.length > 0;
      })
      .join('\n');
  };
})();
