//! The API a script in `browser_exec` is written against.
//!
//! # Two halves, and why they are split this way
//!
//! Reading and rearranging the page is ordinary JavaScript and happens *in* the
//! page: a snapshot is a DOM walk, `text(i)` is `innerText`, `select(i, …)` sets
//! a value. None of it needs the driver.
//!
//! Clicking, typing, pressing a key and navigating are the opposite. A synthetic
//! `element.click()` is a `MouseEvent` with `isTrusted: false`, which sites that
//! care -- payment forms, anything with pointer-event handling -- are entitled to
//! ignore, and rightly do. Those have to go out through the browser's own input
//! pipeline, which only the driver can reach. So the helpers that act on the page
//! *ask the driver*, over the bridge below, and wait for the answer.
//!
//! This is the whole reason the two halves exist: the model writes one script,
//! and the script gets real input without knowing that any of it happened
//! anywhere else.
//!
//! # The bridge
//!
//! `Runtime.addBinding` installs `__pzSend` in the page. Calling it fires an
//! event on the driver's socket; the driver does the work and calls
//! `__pzBridge.resolve` to settle the promise the page is waiting on. So a
//! helper reads as a plain `await`, and the round trip is one hop over a local
//! socket.
//!
//! # Nothing here knows a site
//!
//! No selector for a particular page, no wait tuned for one site, no branch on
//! the host. Everything is a rule about *elements*, so it applies to whatever is
//! loaded. Where a page is unusual, the model adapts in its next call -- that is
//! where per-site knowledge belongs, and it is thrown away when the run ends.

(function () {
  'use strict';

  // Re-entered on every call, and every call but the first is a no-op. A page
  // that has navigated is a new document with no `pz`, which is exactly when it
  // has to be built again.
  if (window.pz) return;

  var send = window.__pzSend;
  if (typeof send !== 'function') {
    throw new Error('the browser bridge is not installed on this page');
  }

  // ---- the bridge ---------------------------------------------------------

  var waiting = new Map();
  var sequence = 0;

  var call = function (request) {
    return new Promise(function (resolve, reject) {
      var id = ++sequence;
      waiting.set(id, { resolve: resolve, reject: reject });
      try {
        send(JSON.stringify({ id: id, op: request.op, args: request.args || {} }));
      } catch (error) {
        waiting.delete(id);
        reject(error);
      }
    });
  };

  /// Settles one waiting helper. Called by the driver, never by a page script.
  window.__pzBridge = {
    resolve: function (id, message, value) {
      var entry = waiting.get(id);
      // Absent when the script was interrupted or the page navigated away; the
      // caller is already gone and re-rejecting it would report a failure for
      // work that no longer has anyone waiting on it.
      if (!entry) return;
      waiting.delete(id);
      if (message) entry.reject(new Error(message));
      else entry.resolve(value);
    }
  };

  function ask(op, args) {
    return call({ op: op, args: args });
  }

  // ---- elements -----------------------------------------------------------

  function element(index) {
    return window.__pzElement(index);
  }

  function collapse(value) {
    return String(value == null ? '' : value)
      .replace(/\s+/g, ' ')
      .trim();
  }

  /// Where to click for this element, in the top page's viewport coordinates.
  ///
  /// Scrolled into view first, because a point outside the viewport is a click
  /// on nothing. Then each enclosing frame's own position is added: a rectangle
  /// measured inside a frame is relative to *that* frame, while the driver
  /// dispatches input in the top page's viewport. Without this, a login form in
  /// an iframe is clicked at the right place on the wrong page.
  function point(index) {
    var target = element(index);
    if (target.scrollIntoView) target.scrollIntoView({ block: 'center', inline: 'center' });
    var rect = target.getBoundingClientRect();
    if (rect.width < 1 && rect.height < 1) {
      throw new Error('element [' + index + '] has no visible box to click');
    }
    var x = rect.left + rect.width / 2;
    var y = rect.top + rect.height / 2;
    var frame = target.ownerDocument.defaultView.frameElement;
    while (frame) {
      var outer = frame.getBoundingClientRect();
      x += outer.left;
      y += outer.top;
      frame = frame.ownerDocument.defaultView.frameElement;
    }
    return { x: Math.round(x), y: Math.round(y) };
  }

  /// Selects everything in a field, so the text that follows replaces it.
  ///
  /// Two paths because a `contenteditable` has no `select()`, and a rich-text
  /// editor is where a careless type-into would append instead of replace.
  function selectAll(target) {
    if (typeof target.select === 'function') {
      target.select();
      return;
    }
    var range = document.createRange();
    range.selectNodeContents(target);
    var selection = window.getSelection();
    selection.removeAllRanges();
    selection.addRange(range);
  }

  /// Everything about one element that a search could reasonably match.
  function haystack(node) {
    var own = '';
    for (var child = node.firstChild; child; child = child.nextSibling) {
      if (child.nodeType === 3) own += ' ' + child.nodeValue;
    }
    var parts = [own, node.getAttribute('aria-label'), node.getAttribute('placeholder'), node.getAttribute('title'), node.getAttribute('alt'), node.getAttribute('name'), node.getAttribute('id'), node.value, node.type];
    // An actionable element is labelled by everything inside it: a row whose
    // subject lives in a child still has that subject as its label, so searching
    // for it finds the row you can click rather than the span inside the row.
    // Wrappers are not given this, or every ancestor above a match would match
    // too and a search would return the whole chain.
    if (window.__pzInteractive(node)) parts.push(node.innerText || '');
    return collapse(parts.filter(Boolean).join(' ')).toLowerCase();
  }

  function labelFor(node) {
    var tag = node.tagName.toLowerCase();
    var text = collapse(node.innerText || '').slice(0, 120);
    var name = node.getAttribute && (node.getAttribute('aria-label') || node.getAttribute('placeholder'));
    return text || (name ? '[' + name + ']' : '') || node.getAttribute('href') || node.type || tag;
  }

  /// A phrase, or `/pattern/flags` for a regular expression.
  function matcherFor(query) {
    var text = String(query);
    var match = /^\/(.*)\/([gimsuy]*)$/.exec(text);
    if (match) {
      var pattern = new RegExp(match[1], match[2].replace('g', ''));
      return function (value) {
        return pattern.test(value);
      };
    }
    var needle = text.toLowerCase();
    return function (value) {
      // Lowered here rather than by each caller: the haystack is a page's own
      // text, which arrives as it was written, and a search that only worked on a
      // pre-lowered copy would quietly miss every capitalised word.
      return String(value).toLowerCase().indexOf(needle) !== -1;
    };
  }

  /// The element a selector names, or a refusal that says which selector failed.
  ///
  /// Thrown rather than returned empty, because every caller is about to act on
  /// the result: a snapshot of nothing and a click on nothing both look like a
  /// page that did not respond, and neither says the selector matched no element.
  function scopeFor(selector) {
    var found = window.__pzQuery(selector, null);
    if (!found) {
      throw new Error('nothing on the page matches ' + JSON.stringify(String(selector)));
    }
    return found;
  }

  /// Text, as much of it as is worth putting in one result.
  ///
  /// Paged rather than truncated, in the same shape `read_file` uses: the lines
  /// that come back are numbered, and the note says which line to ask for next. A
  /// value that fell off the end of a cut string is a value a model cannot tell it
  /// never saw, which is why nothing here silently drops the middle.
  function windowText(whole, options) {
    var limit = options.limit || 300;
    var offset = options.offset || 0;
    var lines = whole.split('\n');
    if (lines.length <= limit && offset === 0) {
      return whole;
    }
    var slice = lines.slice(offset, offset + limit);
    var numbered = [];
    for (var i = 0; i < slice.length; i++) {
      numbered.push(offset + i + 1 + ': ' + slice[i]);
    }
    var remaining = lines.length - (offset + slice.length);
    if (remaining > 0) {
      numbered.push(
        '… ' + remaining + ' more lines. Use findText(text) to search the whole page, pass a selector to read one part, or call text(..., {offset: ' + (offset + slice.length) + '}) for the next page of this one.'
      );
    }
    return numbered.join('\n');
  }

  // ---- the API ------------------------------------------------------------

  var pz = {
    // Reading -----------------------------------------------------------------

    /// The page as numbered text, optionally of one part of it.
    ///
    /// `snapshot()` is the whole page; `snapshot('.thread')` is that part of it,
    /// which is how a long page is read without reading all of it. `"interactive"`
    /// leaves out the prose, and is accepted either alone or as the second
    /// argument beside a selector.
    snapshot: function (target, mode) {
      var wanted = mode;
      var root = null;
      if (typeof target === 'string' && target !== 'interactive' && target !== 'auto') {
        root = scopeFor(target);
      } else if (typeof target === 'string') {
        wanted = target;
      }
      return window.__pzWalk(wanted || 'auto', root);
    },

    /// The visible text of the page, or of one element, or of one part of it.
    ///
    /// A number is an element's number; a string is a selector. Text longer than
    /// a screen is paged rather than cut, because a value that goes missing in the
    /// middle of a blob is a value a model cannot tell it never saw.
    text: function (target, options) {
      if (typeof target === 'number') {
        return window.__pzText(element(target));
      }
      var root = typeof target === 'string' ? scopeFor(target) : document.body || document.documentElement;
      return windowText(window.__pzText(root), options || {});
    },

    /// The markup of an element, or of the page.
    html: function (index) {
      var target =
        index === undefined || index === null ? document.documentElement : element(index);
      return target.outerHTML;
    },

    attr: function (index, name) {
      var value = element(index).getAttribute(String(name));
      return value === null ? '' : value;
    },

    value: function (index) {
      return element(index).value;
    },

    url: function () {
      return location.href;
    },

    title: function () {
      return document.title;
    },

    /// Elements whose own text or attributes match, as `{index, label}` pairs.
    ///
    /// The numbers are the same ones the snapshot prints, because they come from
    /// the same map -- a search is a way of narrowing the tree, not a second
    /// numbering to keep in step with it. Objects rather than lines so a script
    /// can act on what it found without parsing its own search results.
    ///
    /// `{in: selector}` narrows the search to one part of the page, which matters
    /// on a real page: a word like "delete" appears in a menu, a toolbar and the
    /// content, and only one of them is the one meant.
    find: function (query, options) {
      var settings = typeof options === 'number' ? { limit: options } : options || {};
      var matches = matcherFor(query);
      var scope = settings.in ? scopeFor(settings.in) : null;
      var nodes = scope ? window.__pzQueryAll('*', scope) : document.querySelectorAll('*');
      var found = [];
      var most = settings.limit || 25;
      for (var i = 0; i < nodes.length && found.length < most; i++) {
        if (i > 4000) break;
        var node = nodes[i];
        if (!matches(haystack(node))) continue;
        found.push({ index: window.__pzIndex(node), label: labelFor(node) });
      }
      return found;
    },

    /// The lines of the page around each place a value appears.
    ///
    /// The tree answers "what can I act on"; this answers "where does it *say*
    /// that", which is the other half of reading a page and the half a tree is
    /// bad at: a page is mostly prose, and the line a value is on is what a reader
    /// wants. It is the same shape as searching a file, and it returns nothing but
    /// text -- so a search that finds nothing costs almost nothing.
    findText: function (query, options) {
      var settings = options || {};
      var matches = matcherFor(query);
      // The same collected text `text()` returns, not `innerText`: a search that
      // could not see into a shadow root would miss a message body the reader can
      // see, and report "nothing on this page contains …" about a page that does.
      var lines = window.__pzText().split('\n');
      var around = settings.context === undefined ? 2 : settings.context;
      var most = settings.limit || 20;
      var found = [];
      var total = 0;

      for (var i = 0; i < lines.length; i++) {
        if (!matches(lines[i])) continue;
        total++;
        if (found.length >= most) continue;
        var from = Math.max(0, i - around);
        var to = Math.min(lines.length - 1, i + around);
        var block = [];
        for (var j = from; j <= to; j++) {
          block.push((j === i ? '> ' : '  ') + lines[j].trim());
        }
        found.push(block.join('\n'));
        // Past the block just taken: a match inside it is the line already shown
        // with its own context, and printing it twice reads as two places.
        i = to;
      }

      if (!found.length) {
        return 'nothing on this page contains ' + JSON.stringify(String(query));
      }
      var text = found.join('\n---\n');
      if (total > found.length) {
        text += '\n… ' + (total - found.length) + ' more matches. Narrow the search.';
      }
      return text;
    },

    /// One element by selector, shadow roots included.
    ///
    /// Worth its line because `document.querySelector` cannot see into a shadow
    /// root, and on an app built from components the element a script wants is
    /// often inside one.
    query: function (selector, root) {
      return window.__pzQuery(selector, root || null);
    },

    // Acting on the page ------------------------------------------------------

    /// A real click at the element's centre: the browser's own input, trusted.
    click: function (index, options) {
      var where = point(index);
      var count = options && options.count === 2 ? 2 : 1;
      return ask('click', { x: where.x, y: where.y, count: count });
    },

    dblclick: function (index) {
      return pz.click(index, { count: 2 });
    },

    hover: function (index) {
      var where = point(index);
      return ask('hover', { x: where.x, y: where.y });
    },

    focus: function (index) {
      element(index).focus({ preventScroll: true });
      return true;
    },

    /// Types into a field the way a person does, through the browser's input.
    ///
    /// The click first is not decoration: input events go to the focused element,
    /// and a field is focused by being clicked. `replace` is on by default
    /// because a form field usually wants a value rather than an addition to one.
    type: function (index, text, options) {
      var settings = options || {};
      var body = String(text == null ? '' : text);
      var ready = Promise.resolve(true);

      if (index !== undefined && index !== null) {
        var where = point(index);
        ready = ask('click', { x: where.x, y: where.y }).then(function () {
          if (settings.replace !== false) selectAll(element(index));
          return true;
        });
      } else if (settings.replace !== false && document.activeElement) {
        selectAll(document.activeElement);
      }

      return ready
        .then(function () {
          return ask('insertText', { text: body });
        })
        .then(function (result) {
          if (!settings.submit) return result;
          return ask('key', { key: 'Enter' });
        });
    },

    /// A named key, or a combination: `"Enter"`, `"Escape"`, `"Ctrl+A"`.
    press: function (key) {
      return ask('key', { key: String(key) });
    },

    /// Picks an option in a `<select>` by value or by its visible text.
    select: function (index, option) {
      var target = element(index);
      var wanted = String(option);
      var chosen = null;
      for (var i = 0; i < target.options.length; i++) {
        var candidate = target.options[i];
        if (
          candidate.value === wanted ||
          collapse(candidate.textContent).toLowerCase() === wanted.toLowerCase()
        ) {
          chosen = candidate;
          break;
        }
      }
      if (!chosen) {
        var names = [];
        for (var j = 0; j < target.options.length; j++) {
          names.push(collapse(target.options[j].textContent));
        }
        throw new Error(
          'no option matching ' + JSON.stringify(wanted) + ' in [' + index + ']. It has: ' + names.join(', ')
        );
      }
      // Set through the property and then announced, which is the sequence a
      // framework-bound field listens for. Setting the attribute alone leaves a
      // controlled input showing the old value.
      target.value = chosen.value;
      target.dispatchEvent(new Event('input', { bubbles: true }));
      target.dispatchEvent(new Event('change', { bubbles: true }));
      return collapse(chosen.textContent);
    },

    /// Ticks or unticks a box, clicking only when it would change something.
    check: function (index, on) {
      var target = element(index);
      var wanted = on === false ? false : true;
      if (!!target.checked === wanted) return wanted;
      var where = point(index);
      return ask('click', { x: where.x, y: where.y }).then(function () {
        return wanted;
      });
    },

    scroll: function (down, right) {
      return ask('wheel', {
        x: Math.round(window.innerWidth / 2),
        y: Math.round(window.innerHeight / 2),
        dy: down === undefined ? 600 : down,
        dx: right || 0
      });
    },

    scrollTo: function (index) {
      element(index).scrollIntoView({ block: 'center' });
      return true;
    },

    // Getting somewhere -------------------------------------------------------

    /// Navigates, which ends the script: the page it lands on is what comes back.
    goto: function (url) {
      return ask('goto', { url: String(url) });
    },

    back: function () {
      return ask('back', {});
    },

    forward: function () {
      return ask('forward', {});
    },

    reload: function () {
      return ask('reload', {});
    },

    /// Opens a page in a new tab and switches to it.
    newTab: function (url) {
      return ask('newTab', { url: String(url || 'about:blank') });
    },

    // Waiting -----------------------------------------------------------------

    wait: function (ms) {
      return new Promise(function (resolve) {
        setTimeout(resolve, ms || 0);
      });
    },

    /// Resolves when a selector appears or a predicate becomes true.
    waitFor: function (target, timeout) {
      var limit = timeout || 10000;
      var started = Date.now();
      return new Promise(function (resolve, reject) {
        (function poll() {
          var found = null;
          try {
            found = typeof target === 'function' ? target() : document.querySelector(target);
          } catch (error) {
            found = null;
          }
          if (found) {
            resolve(true);
            return;
          }
          if (Date.now() - started > limit) {
            reject(new Error('nothing matched within ' + limit + 'ms'));
            return;
          }
          setTimeout(poll, 200);
        })();
      });
    }
  };

  window.pz = pz;
})();
