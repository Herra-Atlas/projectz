/**
 * The popup: what is happening, the switch, and the two things a person actually wants from
 * a toolbar button -- "read this one" and "leave this site alone".
 *
 * Everything here reads its state from the service worker rather than from storage, because
 * "connected" is a fact only the worker has. It uses `sendMessage` with a reply rather than
 * a push subscription: the popup lives for as long as it is open, so there is nothing to
 * keep in step after it closes.
 */

import { PROTOCOL, hostOf, readableUrl, setupUrl } from "./lib/settings.js";

const elements = {
  power: document.getElementById("power"),
  state: document.getElementById("state"),
  stateText: document.getElementById("stateText"),
  usage: document.getElementById("usage"),
  note: document.getElementById("note"),
  read: document.getElementById("read"),
  readHint: document.getElementById("readHint"),
  site: document.getElementById("site"),
  siteRow: document.getElementById("siteRow"),
  siteLabel: document.getElementById("siteLabel"),
  pairing: document.getElementById("pairing"),
  token: document.getElementById("token"),
  port: document.getElementById("port"),
  pair: document.getElementById("pair"),
  error: document.getElementById("error"),
  setup: document.getElementById("setup"),
  settings: document.getElementById("settings"),
};

/** This tab, which is what "this page" and "this site" mean. */
let tab = null;

const ask = (message) => chrome.runtime.sendMessage(message);
const tell = (message) => chrome.runtime.sendMessage(message).catch(() => undefined);

function paint(status) {
  elements.power.setAttribute("aria-checked", String(Boolean(status.enabled)));

  const state = !status.connected
    ? "waiting"
    : status.enabled
      ? "on"
      : "off";
  elements.state.dataset.state = state;
  elements.stateText.textContent = !status.connected
    ? "Not connected to ProjectZ — open the app"
    : status.enabled
      ? status.pending > 0
        ? "Reading the page…"
        : "On — pages are answered as you read"
      : "Off — nothing is read";

  // What the hour has cost, in the app's figures. Drawn from the app rather than counted here,
  // because the app is what spends and the count it keeps is the one its limit is checked
  // against -- two counters would eventually disagree, and the quieter of the two would win.
  const usage = status.usage || {};
  elements.usage.textContent = status.connected && usage.limit
    ? usage.used === 1
      ? `1 of ${usage.limit} requests used this hour`
      : `${usage.used} of ${usage.limit} requests used this hour`
    : "";

  // A pair out of step is not a failure -- the frames are additive -- but it is the one thing
  // that explains a setting appearing to do nothing, and the fix is always the same.
  const stale = Boolean(status.protocol) && status.protocol !== PROTOCOL;
  elements.note.hidden = !stale;
  elements.note.textContent = stale
    ? "The app and this extension are different versions. Reload the extension from the app's setup page."
    : "";

  elements.pairing.hidden = Boolean(status.connected) || Boolean(status.token);
  elements.token.value = status.token || elements.token.value;
  elements.port.value = String(status.port || 43128);
  elements.error.hidden = !status.lastError;
  elements.error.textContent = status.lastError || "";

  // What the two controls can do on *this* page. A page assist may not read has no site to
  // pause and nothing to look at, so saying so beats two buttons that quietly do nothing --
  // and a `file:` page is readable (if the user allowed it) but has no hostname, so it has
  // nothing for the site switch to name either.
  const url = (tab && tab.url) || "";
  const readable = readableUrl(url, status);
  const host = hostOf(url);
  elements.read.disabled = !readable;
  elements.readHint.textContent = readable
    ? "Look at the page on screen, now, without waiting for it to settle."
    : "This page is not one assist can read — only http, https, and local files if you allow them.";

  const paused = Boolean(host && status.sites && status.sites[host] === "off");
  elements.site.setAttribute("aria-checked", String(!paused));
  elements.siteLabel.textContent = host ? `Read on ${host}` : "Read on this site";
  elements.siteRow.hidden = !host;
}

async function refresh() {
  const [status, [active]] = await Promise.all([
    ask({ type: "status" }),
    chrome.tabs.query({ active: true, currentWindow: true }),
  ]);
  tab = active || null;
  paint(status || {});
}

elements.power.addEventListener("click", async () => {
  const next = elements.power.getAttribute("aria-checked") !== "true";
  // Optimistic, because the app answers on a socket that may not be up: the icon and this
  // switch move at once, and the app's own frame is what settles the truth.
  elements.power.setAttribute("aria-checked", String(next));
  await tell({ type: "setEnabled", enabled: next });
  window.setTimeout(() => void refresh(), 250);
});

elements.read.addEventListener("click", () => {
  void tell({ type: "read" });
  window.close();
});

elements.site.addEventListener("click", async () => {
  const host = tab ? hostOf(tab.url || "") : "";
  if (!host) return;
  const status = await ask({ type: "status" });
  const sites = { ...(status.sites || {}) };
  if (sites[host] === "off") delete sites[host];
  else sites[host] = "off";
  await chrome.storage.local.set({ sites });
  await tell({ type: "settingsChanged" });
  paint({ ...status, sites });
});

elements.pair.addEventListener("click", async () => {
  const token = elements.token.value.trim();
  const port = Number(elements.port.value) || 43128;
  if (token.length < 32) {
    elements.error.hidden = false;
    elements.error.textContent = "That does not look like a pairing token.";
    return;
  }
  await tell({ type: "pair", token, port });
  elements.error.hidden = true;
  window.setTimeout(() => void refresh(), 400);
});

elements.setup.addEventListener("click", () => {
  void chrome.storage.local.get({ port: 43128 }).then(({ port }) => {
    void chrome.tabs.create({ url: setupUrl({ port }) });
  });
});

elements.settings.addEventListener("click", () => chrome.runtime.openOptionsPage());

void refresh();
