import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";

/**
 * Never let a form submit navigate the webview.
 *
 * The app is one page, so there is no server to submit to: a native form
 * submission -- the browser's default action when a `submit` reaches the document
 * with nothing having called `preventDefault` -- navigates to the same URL, which
 * reloads the whole app. The user sees the loading screen and every running reply
 * is gone, which reads as "the app restarted" even though nothing closed. It is
 * easy to trigger without meaning to: a button with no `type` defaults to
 * `submit`, and a form nested inside another form (the question prompt sits inside
 * the composer's form) is invalid HTML whose implicit submission can slip past
 * React's handler.
 *
 * Listening in the capture phase means this runs *before* React's own `onSubmit`,
 * so every real handler still runs exactly as it did -- only the browser's default
 * navigation can no longer happen.
 */
document.addEventListener("submit", (event) => event.preventDefault(), true);

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
