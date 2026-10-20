/**
 * One socket to the app, held open for as long as the browser is running.
 *
 * # Why the socket lives here and not in the page
 *
 * A content script runs in the page's own world, sealed off from the network by the same
 * rules the page is. The service worker is the extension's own context, so it is the only
 * place that can hold a connection to `127.0.0.1` -- and holding *one* is the point: every
 * tab shares this connection, the token is presented once, and a page that navigates does
 * not have to pair again.
 *
 * # Why this stays alive
 *
 * A service worker is meant to be evicted when idle, which would drop the socket every
 * half minute. Chrome keeps an extension's service worker alive for as long as it holds
 * an open WebSocket (`minimum_chrome_version` 116 in the manifest is what buys this), so
 * the connection is both the transport and the reason the worker survives. A dropped
 * socket is not a lost cause: the reconnect below retries with a growing pause, and every
 * message a content script sends wakes the worker to try again.
 *
 * # Why a reply is pushed to the tab that asked
 *
 * The app may spend seconds on a model, and a service worker that is evicted mid-request
 * takes any `sendResponse` promise with it. So the content script does not wait: it says
 * what is on the page, and cards arrive later as their own message. A lost reply costs a
 * card, never a page that hangs.
 */

import { DEFAULTS, PROTOCOL, load, readableUrl, save, socketUrl } from "./lib/settings.js";

/**
 * The state, read from storage without blocking the listeners below.
 *
 * **Nothing may be awaited before the listeners are registered.** A service worker is
 * started *by* an event, and an event that arrives while the module's top-level await is
 * still pending has nowhere to go -- the worker looks dead, and the page that sent it waits
 * forever. So the default record is in place synchronously and the stored one replaces it
 * the moment it arrives; anything that needs the stored values waits on `ready`.
 */
let state = { ...DEFAULTS };
const ready = load().then((stored) => {
  state = stored;
  return stored;
});

/** The live socket, or null. */
let socket = null;
/** Whether a reconnect is already scheduled, so a burst of failures does not stack timers. */
let reconnecting = false;
/** The pause before the next attempt. Doubles up to a ceiling. */
let backoff = 1000;
/** In-flight page requests: id -> the tab that should receive the cards. */
const waiting = new Map();

// ---------------------------------------------------------------------------------------
// The socket
// ---------------------------------------------------------------------------------------

function send(frame) {
  if (!socket || socket.readyState !== WebSocket.OPEN) return false;
  socket.send(JSON.stringify(frame));
  return true;
}

function connect() {
  if (socket && (socket.readyState === WebSocket.OPEN || socket.readyState === WebSocket.CONNECTING)) {
    return;
  }
  if (!state.token) {
    // Nothing to present means nothing to connect to. The popup says so and offers the
    // pairing field, so there is no point retrying until something changes.
    void refreshBadge();
    return;
  }
  let opened;
  try {
    opened = new WebSocket(socketUrl(state));
  } catch {
    scheduleReconnect();
    return;
  }
  socket = opened;

  opened.addEventListener("open", () => {
    backoff = 1000;
    // The first frame is the pairing: name, token, and which profile this is, so a user
    // with several browsers can tell them apart in the app.
    send({
      type: "hello",
      token: state.token,
      client: { profile: profileName(), version: chrome.runtime.getManifest().version },
    });
  });

  opened.addEventListener("message", (event) => {
    let frame;
    try {
      frame = JSON.parse(event.data);
    } catch {
      return;
    }
    void handle(frame);
  });

  opened.addEventListener("close", () => {
    socket = null;
    state = { ...state, connected: false };
    void save({ connected: false });
    void refreshBadge();
    scheduleReconnect();
  });

  // An error is followed by a close, which is where the retry is scheduled. Handled so
  // the console does not fill with uncaught events on a machine where the app is shut.
  opened.addEventListener("error", () => {});
}

function scheduleReconnect() {
  if (reconnecting) return;
  reconnecting = true;
  setTimeout(() => {
    reconnecting = false;
    connect();
  }, backoff);
  backoff = Math.min(backoff * 2, 30000);
}

/**
 * The facts the app owns, as one patch of the stored record.
 *
 * Together because they arrive together and are read together: a popup that showed one hour's
 * limit beside another hour's spending would be worse than showing neither. A frame that
 * carries none of them -- an `error`, or a `cards` from an app built before this -- yields an
 * empty patch rather than overwriting a mirror with `undefined`.
 */
function mirrorOf(frame) {
  const patch = {};
  if (frame.limits && typeof frame.limits === "object") patch.limits = frame.limits;
  if (frame.usage && typeof frame.usage === "object") patch.usage = frame.usage;
  if (typeof frame.protocol === "number") patch.protocol = frame.protocol;
  return patch;
}

async function handle(frame) {
  if (frame.type === "welcome") {
    const mirror = mirrorOf(frame);
    if (mirror.protocol && mirror.protocol !== PROTOCOL) {
      // Not a failure: the frames are additive, so an older pair keeps working. It is the one
      // thing that explains a setting which seems to do nothing -- and the fix is always the
      // same, so it is named here rather than left to be guessed at.
      console.info(
        `[assist] the app speaks protocol ${mirror.protocol} and this extension speaks ${PROTOCOL}; reload the extension from the app's setup page.`
      );
    }
    state = {
      ...state,
      ...mirror,
      connected: true,
      enabled: Boolean(frame.enabled),
      display: frame.display !== false,
    };
    await save({ ...mirror, connected: true, enabled: state.enabled, display: state.display });
    await refreshBadge();
    // A page opened while the app was shut has nothing pending, so the tabs are told the
    // state changed and one of them may now be worth reading.
    void broadcast({ type: "state", enabled: state.enabled, display: state.display });
    return;
  }
  if (frame.type === "state") {
    const mirror = mirrorOf(frame);
    state = { ...state, ...mirror, enabled: Boolean(frame.enabled) };
    await save({ ...mirror, enabled: state.enabled });
    await refreshBadge();
    void broadcast({ type: "state", enabled: state.enabled, display: state.display });
    return;
  }
  if (frame.type === "cards") {
    const mirror = mirrorOf(frame);
    if (Object.keys(mirror).length > 0) {
      // What the look cost, which is the number the popup draws -- so it moves with the work
      // rather than on a poll of its own.
      state = { ...state, ...mirror };
      await save(mirror);
    }
    const tabId = waiting.get(frame.id);
    waiting.delete(frame.id);
    if (typeof tabId === "number") {
      // Failing to reach the tab is normal -- it was closed, or navigated away -- and is
      // not worth reporting to anyone.
      chrome.tabs.sendMessage(tabId, { type: "cards", ...frame }).catch(() => undefined);
    }
    await refreshBadge();
    return;
  }
  if (frame.type === "error") {
    state = { ...state, lastError: frame.message };
    await refreshBadge(frame.message);
  }
}

/** A name for this browser profile, so the app can say which one is attached. */
function profileName() {
  // There is no API for a profile's name. The user agent's platform plus the extension id's
  // last four characters is enough to tell two profiles on one machine apart.
  const id = chrome.runtime.id || "";
  return `${navigator.platform || "browser"} ${id.slice(-4)}`;
}

/** Tells every tab what changed. */
async function broadcast(message) {
  const tabs = await chrome.tabs.query({});
  for (const tab of tabs) {
    if (typeof tab.id !== "number") continue;
    chrome.tabs.sendMessage(tab.id, message).catch(() => undefined);
  }
}

// ---------------------------------------------------------------------------------------
// The icon
// ---------------------------------------------------------------------------------------

/**
 * The icon is the readout.
 *
 * The colour says whether assist is on -- the switch can be moved from the tray while no
 * tab is looking, so it has to be visible without opening anything -- and the badge says
 * whether a request is in flight, which is the only part of this that takes time.
 */
async function refreshBadge(errorText) {
  const on = Boolean(state.enabled) && state.connected;
  const suffix = on ? "on" : "off";
  await chrome.action.setIcon({
    path: {
      16: `icons/${suffix}-16.png`,
      32: `icons/${suffix}-32.png`,
      48: `icons/${suffix}-48.png`,
      128: `icons/${suffix}-128.png`,
    },
  });
  await chrome.action.setBadgeText({ text: waiting.size > 0 ? "…" : "" });
  if (waiting.size > 0 || on) {
    await chrome.action.setBadgeBackgroundColor({ color: on ? "#b26b18" : "#8a8a93" });
  }
  if (errorText) {
    await chrome.action.setTitle({ title: `ProjectZ Assist — ${errorText}` });
  } else if (!state.connected) {
    await chrome.action.setTitle({ title: "ProjectZ Assist — not connected to ProjectZ" });
  } else {
    await chrome.action.setTitle({ title: `ProjectZ Assist — ${state.enabled ? "on" : "off"}` });
  }
}

// ---------------------------------------------------------------------------------------
// Messages from the extension's own pages
// ---------------------------------------------------------------------------------------

chrome.runtime.onMessage.addListener((message, sender, respond) => {
  if (!message || typeof message.type !== "string") return false;

  if (message.type === "page") {
    const tabId = sender.tab && sender.tab.id;
    if (typeof tabId !== "number") return false;
    const frame = { ...message.frame, id: message.frame.id || `${tabId}-${Date.now()}` };
    waiting.set(frame.id, tabId);
    if (!send({ type: "page", id: frame.id, frame })) {
      waiting.delete(frame.id);
      // The app is not there. Told plainly rather than left in a "thinking" state that
      // never resolves.
      chrome.tabs
        .sendMessage(tabId, { type: "offline", id: frame.id })
        .catch(() => undefined);
      connect();
      return false;
    }
    void refreshBadge();
    return false;
  }

  if (message.type === "setEnabled") {
    const wanted = Boolean(message.enabled);
    state = { ...state, enabled: wanted };
    // Written before the app confirms, so the icon moves the instant it is pressed; the
    // app's own `state` frame is what settles it if the two ever disagree.
    void save({ enabled: wanted }).then(() => refreshBadge());
    send({ type: "setEnabled", enabled: wanted });
    return false;
  }

  if (message.type === "pair") {
    void ready.then(() => {
      state = { ...state, token: String(message.token || "").trim(), port: Number(message.port) || 43128 };
      return save({ token: state.token, port: state.port });
    }).then(() => {
      // A new address needs a new socket: the old one is either dead or pointed at
      // somewhere else, and keeping it would mean pairing that appears to do nothing.
      if (socket) socket.close();
      socket = null;
      backoff = 1000;
      connect();
    });
    return false;
  }

  if (message.type === "status") {
    // The popup asks about the record as soon as it opens, which can be before the stored
    // one has arrived -- so the answer waits for it rather than reporting the defaults.
    void ready.then(() => {
      respond({
        connected: state.connected,
        enabled: state.enabled,
        display: state.display,
        token: state.token,
        port: state.port,
        pending: waiting.size,
        lastError: state.lastError || "",
        sites: state.sites,
        // Read rather than re-derived by the popup: `files` decides whether a `file://` page
        // may be read at all, and the two mirrors are what the popup draws its figures from.
        files: state.files,
        limits: state.limits,
        usage: state.usage,
        protocol: state.protocol,
      });
    });
    return true;
  }

  if (message.type === "read") {
    // The popup's button: asks the tab on screen to look now, whatever the automatic
    // rules would have said.
    chrome.tabs
      .query({ active: true, currentWindow: true })
      .then(([tab]) => {
        if (!tab || typeof tab.id !== "number") return;
        // The scheme is checked here as well as in the page, because this is the one path that
        // reaches a tab without the page's own script having decided anything: a browser
        // settings page or a devtools window has no content script to answer, and asking it
        // anyway would look like a button that does nothing.
        if (!readableUrl(tab.url || "", state)) {
          state = { ...state, lastError: "This page is not one assist can read." };
          void refreshBadge(state.lastError);
          return;
        }
        chrome.tabs.sendMessage(tab.id, { type: "read" }).catch(() => undefined);
      })
      .catch(() => undefined);
    return false;
  }

  if (message.type === "settingsChanged") {
    void load().then((next) => {
      state = next;
      void refreshBadge();
      void broadcast({ type: "state", enabled: state.enabled, display: state.display });
    });
    return false;
  }

  return false;
});

// ---------------------------------------------------------------------------------------
// Where it starts
// ---------------------------------------------------------------------------------------

/** What every entry point does once the stored record has arrived: connect, and paint. */
function start() {
  void ready.then(() => {
    connect();
    return refreshBadge();
  });
}

// Both are needed: `onInstalled` fires when the extension is loaded (including from the
// app's setup page) and `onStartup` when the browser comes back with it already in place.
chrome.runtime.onInstalled.addListener(start);
chrome.runtime.onStartup.addListener(start);
chrome.storage.onChanged.addListener((changes) => {
  if (changes.token || changes.port) {
    void load().then((next) => {
      if (next.token !== state.token || next.port !== state.port) {
        state = next;
        if (socket) socket.close();
        socket = null;
        connect();
      }
    });
  }
});

// The worker may be woken by a message rather than by one of the events above, in which case
// this is the first thing that runs and the socket is not up yet.
start();
