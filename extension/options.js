/**
 * The extension's own settings.
 *
 * Every change writes the shared storage record and then tells the service worker, because the
 * worker is the thing holding the socket and the only thing that can push the new rules to the
 * tabs that are already open. A settings page that saved silently would leave a page being read
 * under the rules that were in force when it loaded.
 *
 * What belongs here, and what does not, is the split the whole feature is built on: this page
 * owns *the browser's* half -- which pages are read, how much of one may leave, and how an
 * answer is drawn -- while everything about what is worth answering, by which model and how
 * often, lives in ProjectZ. The two facts taken from the app that appear here (its floor, and
 * this hour's spending) are read-only, and are drawn here because the browser is where someone
 * is when they wonder why nothing happened.
 */

import { PROTOCOL, load, save, hostOf, setupUrl } from "./lib/settings.js";

const $ = (id) => document.getElementById(id);
let state = await load();

/** Writes a patch, and tells the worker so open tabs learn the new rules. */
async function patch(changes) {
  state = { ...state, ...changes };
  await save(changes);
  await chrome.runtime.sendMessage({ type: "settingsChanged" }).catch(() => undefined);
}

// ---------------------------------------------------------------------------------------
// The sections
// ---------------------------------------------------------------------------------------

const SECTIONS = ["reading", "sending", "answers", "connection"];

/**
 * Shows one section and marks its tab.
 *
 * Switched rather than scrolled to, and kept in the URL: a settings page is opened to find one
 * row, and the hash is what makes a reload or a bookmark land back on the section it was on.
 */
function openSection(id, remember = true) {
  const wanted = SECTIONS.includes(id) ? id : SECTIONS[0];
  for (const section of document.querySelectorAll("main > section")) {
    section.toggleAttribute("data-open", section.id === wanted);
  }
  for (const button of $("tabs").querySelectorAll("button")) {
    button.setAttribute("aria-current", String(button.dataset.section === wanted));
  }
  if (remember) history.replaceState(null, "", `#${wanted}`);
}

$("tabs").addEventListener("click", (event) => {
  const button = event.target.closest("button[data-section]");
  if (button) openSection(button.dataset.section);
});

// ---------------------------------------------------------------------------------------
// Drawing the state
// ---------------------------------------------------------------------------------------

/** A segmented control: one of a few values, always showing which one is in force. */
function segmented(container, values, current, pick) {
  container.replaceChildren(
    ...values.map(({ value, label }) => {
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = label;
      button.dataset.value = value;
      button.setAttribute("aria-pressed", String(String(value) === String(current)));
      button.addEventListener("click", () => void pick(value));
      return button;
    })
  );
}

function paintConnection() {
  const connected = Boolean(state.connected);
  $("status").dataset.state = connected ? (state.enabled ? "on" : "waiting") : "waiting";
  $("statusText").textContent = connected ? (state.enabled ? "On" : "Off") : "Not connected";
  $("connection").textContent = connected
    ? state.enabled
      ? "Connected — assist is on"
      : "Connected — assist is switched off in ProjectZ"
    : "Not connected. Is ProjectZ running?";

  const stale = Boolean(state.protocol) && state.protocol !== PROTOCOL;
  const version = $("version");
  version.textContent = stale
    ? `Version ${chrome.runtime.getManifest().version}. The app speaks protocol ${state.protocol} and this extension speaks ${PROTOCOL} — reload the extension from the app's setup page.`
    : `Version ${chrome.runtime.getManifest().version}.`;
  version.dataset.warning = String(stale);
}

function paintSites() {
  const hosts = Object.keys(state.sites || {}).sort();
  const list = $("sites");
  if (hosts.length === 0) {
    const empty = document.createElement("div");
    empty.className = "row";
    const label = document.createElement("div");
    label.className = "label";
    const strong = document.createElement("strong");
    strong.textContent = "Every site is read";
    const span = document.createElement("span");
    span.textContent = "Add one to leave it out, or use the popup's switch on the site itself.";
    label.append(strong, span);
    empty.append(label);
    list.replaceChildren(empty);
    return;
  }
  list.replaceChildren(
    ...hosts.map((host) => {
      const row = document.createElement("div");
      row.className = "row";
      const wrap = document.createElement("div");
      wrap.className = "host";
      const name = document.createElement("code");
      name.textContent = host;
      const remove = document.createElement("button");
      remove.type = "button";
      remove.className = "quiet";
      remove.textContent = "Read again";
      remove.addEventListener("click", () => {
        const sites = { ...state.sites };
        delete sites[host];
        void patch({ sites }).then(paintSites);
      });
      wrap.append(name, remove);
      row.append(wrap);
      return row;
    })
  );
}

/**
 * What the app will read, in the app's own numbers.
 *
 * Shown and never editable: the floor is what the app ignores a block under, so a copy of it
 * here would be a second answer to a question only ProjectZ can answer. It is on this page
 * because "why is nothing happening?" is asked while looking at a page, and the answer is often
 * a floor nobody can see from the browser.
 */
function paintAppLimits() {
  const limits = state.limits || {};
  $("appLimits").textContent = state.protocol
    ? `ProjectZ ignores a block under ${limits.minChars} characters and reads at most ${limits.maxAskChars} of each one. It owns both numbers; they are changed in its Settings.`
    : "ProjectZ's own limits are not known yet — they arrive with the first connection. While it is not running, the two numbers above are the only ones in force.";
}

function paintStyle() {
  const inline = state.cardStyle === "inline";
  $("styleHint").textContent = inline
    ? "An aside appended to the text, dimmed."
    : "A card beside the text it answers.";
  // Three rows that only mean something for a card. Hidden rather than disabled: a control that
  // cannot do anything is worse than one that is not there.
  for (const id of ["placementRow", "opacityRow", "hideAfterRow"]) {
    $(id).hidden = inline;
  }
}

function paintOpacity(value) {
  $("opacity").value = String(value);
  $("opacitySample").style.setProperty("--projectz-solid", String(value));
}

function paintSettle() {
  for (const button of $("settle").querySelectorAll("button")) {
    button.setAttribute("aria-pressed", String(Number(button.dataset.value) === Number(state.settle)));
  }
}

function paintCard() {
  for (const button of $("card").querySelectorAll("button")) {
    button.setAttribute("aria-pressed", String(button.dataset.value === state.card));
  }
}

function paintStyleButtons() {
  const current = state.cardStyle === "inline" ? "inline" : "card";
  for (const button of $("style").querySelectorAll("button")) {
    button.setAttribute("aria-pressed", String(button.dataset.value === current));
  }
}

/**
 * Whether Chrome has been told to let this extension into local files.
 *
 * Two switches have to agree before a `file://` page is read: this page's own, and the browser's,
 * which lives on the extension's details page and which nothing here can set. A row saying "on"
 * while the browser is still refusing is the most confusing state this feature has, so the
 * disagreement is named -- in the browser's own words, because they are what the user has to
 * find on a page this extension cannot open for them.
 */
function paintFileGrant() {
  const note = $("fileGrant");
  if (!state.files) {
    note.hidden = true;
    return;
  }
  note.hidden = false;
  note.dataset.warning = "false";
  if (typeof chrome.extension?.isAllowedFileSchemeAccess !== "function") {
    // A browser that does not answer the question: the instruction is all that can be said, and
    // leaving the row on "checking" forever would be worse than saying less.
    note.textContent = "Local files also need “Allow access to file URLs” on this extension's details page.";
    return;
  }
  note.textContent = "Checking whether Chrome allows this…";
  chrome.extension.isAllowedFileSchemeAccess((allowed) => {
    note.dataset.warning = String(!allowed);
    note.textContent = allowed
      ? "Chrome allows this extension to read local files."
      : "Chrome has not allowed this yet: open the extension's details page and turn on “Allow access to file URLs”. Without it, this script is not even injected into a local file.";
  });
}

function paintAll() {
  $("auto").setAttribute("aria-checked", String(Boolean(state.auto)));
  $("files").setAttribute("aria-checked", String(Boolean(state.files)));
  $("port").value = String(state.port || 43128);
  $("token").value = state.token || "";
  $("hideAfter").value = String(state.hideAfter || 0);
  $("maxBlocks").value = String(state.maxBlocks || 60);
  $("maxBlockChars").value = String(state.maxBlockChars || 600);
  paintOpacity(Number(state.opacity) || 0.88);

  segmented($("settle"), [
    { value: 400, label: "Briefly" },
    { value: 900, label: "A moment" },
    { value: 2000, label: "Longer" },
  ], state.settle, (value) => patch({ settle: Number(value) }).then(paintSettle));

  segmented($("card"), [
    { value: "auto", label: "Nearest" },
    { value: "above", label: "Above" },
    { value: "below", label: "Below" },
  ], state.card, (value) => patch({ card: String(value) }).then(paintCard));

  segmented($("style"), [
    { value: "card", label: "Card" },
    { value: "inline", label: "Beside the text" },
  ], state.cardStyle, (value) =>
    patch({ cardStyle: String(value) }).then(() => {
      paintStyleButtons();
      paintStyle();
    })
  );

  paintSettle();
  paintCard();
  paintStyleButtons();
  paintStyle();
  paintSites();
  paintAppLimits();
  paintConnection();
  paintFileGrant();

  // The section in the URL wins over the first one, so a bookmark opens where it was left.
  openSection(location.hash.replace("#", ""), false);
}

// ---------------------------------------------------------------------------------------
// The controls
// ---------------------------------------------------------------------------------------

$("auto").addEventListener("click", () => {
  const next = $("auto").getAttribute("aria-checked") !== "true";
  $("auto").setAttribute("aria-checked", String(next));
  void patch({ auto: next });
});

$("files").addEventListener("click", () => {
  const next = $("files").getAttribute("aria-checked") !== "true";
  $("files").setAttribute("aria-checked", String(next));
  void patch({ files: next }).then(paintFileGrant);
});

$("hideAfter").addEventListener("change", () => {
  const seconds = Math.max(0, Math.min(120, Number($("hideAfter").value) || 0));
  $("hideAfter").value = String(seconds);
  void patch({ hideAfter: seconds });
});

// Written when the drag ends rather than while it moves: a slider reports every pixel of a drag,
// and a storage write per pixel would be a write per frame for a number nobody reads until it
// stops. The sample follows the thumb either way, because that is what it is for.
$("opacity").addEventListener("input", () => paintOpacity($("opacity").value));
$("opacity").addEventListener("change", () => {
  const value = Math.max(0.6, Math.min(1, Number($("opacity").value) || 0.88));
  paintOpacity(value);
  void patch({ opacity: value });
});

$("maxBlocks").addEventListener("change", () => {
  const value = Math.max(10, Math.min(120, Number($("maxBlocks").value) || 60));
  $("maxBlocks").value = String(value);
  void patch({ maxBlocks: value });
});

$("maxBlockChars").addEventListener("change", () => {
  const value = Math.max(200, Math.min(2000, Number($("maxBlockChars").value) || 600));
  $("maxBlockChars").value = String(value);
  void patch({ maxBlockChars: value });
});

$("port").addEventListener("change", () => {
  const port = Math.max(1, Math.min(65535, Number($("port").value) || 43128));
  $("port").value = String(port);
  void patch({ port });
});

$("pair").addEventListener("click", async () => {
  const token = $("token").value.trim();
  if (token.length < 32) {
    $("token").focus();
    return;
  }
  await patch({ token, port: Number($("port").value) || 43128 });
  // The worker reconnects on a pair, and its next status is the answer to whether it
  // worked -- so this waits briefly and re-reads rather than claiming success itself.
  window.setTimeout(async () => {
    state = await load();
    paintConnection();
  }, 600);
});

$("add").addEventListener("click", () => {
  const host = $("host").value.trim().toLowerCase().replace(/^https?:\/\//, "").split("/")[0];
  if (!host) return;
  $("host").value = "";
  void patch({ sites: { ...state.sites, [host]: "off" } }).then(paintSites);
});

$("host").addEventListener("keydown", (event) => {
  if (event.key === "Enter") $("add").click();
});

$("setup").addEventListener("click", () => {
  void chrome.tabs.create({ url: setupUrl(state) });
});

// The status and the switch live in the app, so this page has to follow them rather than
// own them: a settings tab left open while the tray's item is clicked would otherwise show
// a state that stopped being true.
chrome.storage.onChanged.addListener(() => {
  void load().then((next) => {
    state = next;
    paintConnection();
    paintAppLimits();
  });
});

paintAll();
void hostOf(location.href);
