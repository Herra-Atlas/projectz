//! What the app takes away from the embedded webview.
//!
//! Two engine behaviours would otherwise be drawn *instead of* the app's own:
//! the built-in find bar and the browser's default right-click menu. Both are
//! host UI, not page UI -- no `keydown` or `contextmenu` handler can suppress
//! them reliably -- so they are turned off here, once, before the window is
//! shown.
//!
//! # Why the find bar is off
//!
//! Ctrl+F inside WebView2 is a *browser* accelerator: the engine opens its own
//! find bar, and `preventDefault` cannot reach a shortcut the host answers
//! before the renderer sees it. The app has its own find (`src/features/find/`),
//! which knows which surfaces are searchable, fades everything in them but the
//! matches and scrolls to the current one -- so leaving the engine's bar in place
//! would mean two bars for one keypress, the first of which dims nothing and
//! searches the sidebar.
//!
//! `AreBrowserAcceleratorKeysEnabled` off turns off exactly the browser half:
//! Find (Ctrl+F, F3), Print, Reload, zoom and the DevTools keys. The *editing*
//! keys are documented as unaffected -- Ctrl+A, C, V, X, Z, Home/End/PageUp/Down
//! -- so the composer, the file editor and every text field keep working. Losing
//! Reload and the zoom keys in a desktop app costs nothing; Ctrl+Shift+I still
//! opens DevTools from Tauri's own debug menu in a debug build.
//!
//! # Why the engine's menu is off
//!
//! Every context menu this app draws is its own: the session list, the files
//! tree, the blank space under it. Everywhere else -- the transcript, Settings,
//! the job editor -- a right-click fell through to Edge's menu, offering Reload,
//! Save as, Print and Inspect on a page that is not a document. That menu cannot
//! be themed or made to describe this app, so it goes, and a right-click where no
//! surface has drawn a menu now does nothing. Doing nothing is the quieter
//! gesture: a menu of browser actions over a conversation is a worse answer than
//! no answer.
//!
//! **The cost, stated plainly:** the engine's menu was also the pointer route to
//! Cut/Copy/Paste in a text field. Those still work from the keyboard and from
//! the app's own selection actions; what is gone is the generic menu that used to
//! appear for them. The panel's embedded browser is a separate webview and keeps
//! its own menu, because there the page really is a page.
//!
//! Windows only. WKWebView and WebKitGTK have neither a find bar nor a browser
//! context menu to suppress, so the frontend's handlers are the whole mechanism
//! on those platforms.

use tauri::WebviewWindow;

/// Applies both settings to the main window's webview.
///
/// Called once from `setup`. Every failure is logged and then ignored: an old
/// WebView2 runtime that cannot expose a setting keeps the engine's behaviour for
/// it, and the app's own UI still works beside it. Nothing here is worth a window
/// that does not open.
#[cfg(windows)]
pub fn apply(window: &WebviewWindow) {
    let result = window.with_webview(|webview| {
        use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings3;
        use windows_core::Interface;

        // SAFETY: these are the documented calls on the live controller of this
        // window's own webview, and every result is checked. The cast can fail on
        // a runtime that predates the interface, which is handled rather than
        // assumed.
        unsafe {
            let Ok(core) = webview.controller().CoreWebView2() else {
                tracing::warn!("could not reach the webview's core");
                return;
            };
            let Ok(settings) = core.Settings() else {
                tracing::warn!("could not read the webview's settings");
                return;
            };

            // On the base settings interface, so no cast is needed.
            if let Err(error) = settings.SetAreDefaultContextMenusEnabled(false) {
                tracing::warn!("could not turn off the webview's context menu: {error}");
            }

            match settings.cast::<ICoreWebView2Settings3>() {
                Ok(settings) => {
                    if let Err(error) = settings.SetAreBrowserAcceleratorKeysEnabled(false) {
                        tracing::warn!("could not turn off the webview's browser keys: {error}");
                    }
                }
                Err(_) => {
                    tracing::warn!(
                        "this WebView2 runtime predates the browser-accelerator setting"
                    );
                }
            }
        }
    });
    if let Err(error) = result {
        tracing::warn!("could not reach the webview: {error}");
    }
}

/// Nothing to do: only WebView2 has a find bar and a browser menu of its own.
#[cfg(not(windows))]
pub fn apply(_window: &WebviewWindow) {}
