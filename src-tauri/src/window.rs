//! Where the window was, and how big it was.
//!
//! # Why a setting rather than the official plugin
//!
//! `tauri-plugin-window-state` does this job, and would be less code here. It was
//! not used because it writes its own JSON file into the app *config* directory,
//! and ProjectZ keeps every other piece of durable state in `projectz.sqlite3`
//! under `settings`. Two stores means two questions -- "what is my config
//! directory on this machine" and "which one was actually written" -- for a
//! single integer. Reading through [`Database::setting`] and writing through
//! [`Database::set_setting`] keeps one storage story, and makes the value
//! readable from the Database page the app already ships.
//!
//! The plugin is also strictly better on one axis: it tracks the window across
//! a *display change* by re-validating on `ScaleFactorChanged`. The behaviour
//! implemented here is the same idea reached without the dependency --
//! [`clamp`] is called against the live monitor list on every launch.
//!
//! # Why the window is created hidden
//!
//! A window restored from `tauri.conf.json` appears at its configured 800x600 and
//! then jumps. That is the one thing a "remembers where it was" feature must not
//! do, so `visible` is false there and the real geometry is applied in
//! [`restore`] before the first `show()`. The user sees the window arrive once,
//! already where they left it. The cost is that a window which fails to restore
//! is invisible rather than wrongly placed, which is why every restore path ends
//! in `show()` and there is no early return that skips it.
//!
//! # Why geometry is stored in logical units
//!
//! Physical pixels are what the OS reports, and they are meaningless across
//! machines: a 1600-wide window on a 150% display is 1600 *logical* pixels, and
//! storing 2400 physical would restore it at 160% of the intended size on a
//! 100% display. Logical units survive the move, so they are what is stored, and
//! Tauri converts through [`LogicalPosition`]/[`LogicalSize`] at the call.
//!
//! # Why the maximized flag is stored separately
//!
//! A maximized window's reported size is the screen, so storing the live
//! geometry while maximized would make "un-maximize" restore to fullscreen --
//! and on a changed display layout, to the wrong fullscreen. The last
//! *non-maximized* rect is therefore kept separately and it is that which is
//! restored; `maximized` only records how to show the window afterwards.

use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, WebviewWindow, WindowEvent};

use crate::database::Database;

/// The settings key this lives under.
pub const WINDOW_KEY: &str = "app.window";

/// The label of the window whose geometry is tracked.
const WINDOW_LABEL: &str = "main";

/// How long the window must sit still before its geometry is written.
///
/// A drag raises `Moved` once per frame, so writing on every event would be
/// hundreds of writes for one gesture. The timer is restarted by each new
/// event, so the write happens once the user lets go.
const PERSIST_DEBOUNCE_MS: u64 = 400;

/// The smallest window we will open at.
///
/// A stored width of zero -- or a value that became nonsense after a bad write
/// -- would otherwise produce an invisible window with no way to recover it.
const MIN_WIDTH: f64 = 320.0;
const MIN_HEIGHT: f64 = 240.0;

/// How much of the window must be on the target monitor for its position to be
/// honoured, as a fraction of the window's own area.
///
/// Anything at all is not enough. A sliver of title bar in the corner is
/// technically visible but effectively unreachable, and the user has no way to
/// discover where the window went. Half is the line: below it the window is
/// centred back onto the monitor, above it the position is left as the user
/// left it. Deliberately permissive -- a window nudged slightly off the edge is
/// still perfectly usable, and snapping it back would feel like the window was
/// fighting the user.
const MIN_VISIBLE_FRACTION: f64 = 0.5;

/// The window's position and size, in logical pixels.
///
/// `#[serde(default)]` on every field so a value written by an older build, or
/// one missing a key, still reads -- a malformed row must never be the reason
/// the app fails to launch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default)]
    pub width: f64,
    #[serde(default)]
    pub height: f64,
    /// Whether the window was maximized. Restored after the normal rect, so
    /// un-maximizing returns to a usable size rather than to the screen.
    #[serde(default)]
    pub maximized: bool,
}

/// A monitor's usable area, in the same logical units as [`WindowState`].
///
/// A plain struct rather than `tauri::window::Monitor` because `clamp` is pure
/// and must be testable without a display attached; the live monitor is
/// converted into one of these at the call site.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Area {
    /// Builds an area from a monitor's work area and scale factor.
    ///
    /// The *work area* rather than the full monitor, so a restored window is
    /// not placed under the taskbar it was last seen under.
    fn from_monitor(monitor: &tauri::window::Monitor) -> Self {
        let scale = monitor.scale_factor();
        let work = monitor.work_area();
        Area {
            x: work.position.x as f64 / scale,
            y: work.position.y as f64 / scale,
            width: work.size.width as f64 / scale,
            height: work.size.height as f64 / scale,
        }
    }

    fn right(&self) -> f64 {
        self.x + self.width
    }

    fn bottom(&self) -> f64 {
        self.y + self.height
    }

    /// How much of `rect` lies inside this area.
    ///
    /// Measured on the intersection rather than by testing whether the centre
    /// is contained: a window that straddles a monitor boundary is on *both*,
    /// and the overlap is what says whether any of it can be reached.
    fn overlap(&self, rect: &WindowState) -> f64 {
        let width = (self.right() - rect.x).min(rect.right() - self.x).max(0.0);
        let height = (self.bottom() - rect.y)
            .min(rect.bottom() - self.y)
            .max(0.0);
        width * height
    }

    /// Whether enough of `rect` is on this monitor for the user to get at it.
    ///
    /// Measured as a *fraction of the window's own area* rather than a fixed
    /// number of pixels. A fixed threshold gets both edges wrong: 96px of
    /// visible sliver is far more of a small window than of a large one, and a
    /// window 120px off a 1920px screen is more usable than a window 120px off a
    /// 320px one. A ratio asks the question that actually matters -- can the user
    /// see and drag most of this thing.
    fn holds(&self, rect: &WindowState) -> bool {
        let area = rect.width * rect.height;
        if area <= 0.0 {
            return false;
        }
        self.overlap(rect) / area >= MIN_VISIBLE_FRACTION
    }
}

impl WindowState {
    pub fn right(&self) -> f64 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.height
    }

    /// Whether the value describes a window that could actually be opened.
    ///
    /// Used before saving as well as before restoring: a stored width of zero
    /// would otherwise be written back on every launch, replacing a good value
    /// with one that still cannot be restored.
    fn is_usable(&self) -> bool {
        self.width.is_finite()
            && self.height.is_finite()
            && self.x.is_finite()
            && self.y.is_finite()
            && self.width >= MIN_WIDTH
            && self.height >= MIN_HEIGHT
    }
}

/// Chooses the monitor a saved rect should open on.
///
/// The one whose work area holds most of the rect, so a window that straddles
/// two monitors is claimed by whichever it mostly occupies rather than by
/// whichever happens to be listed first. Returns `None` when the rect touches
/// no monitor at all, which is the undocked-laptop case.
fn target_monitor(rect: &WindowState, monitors: &[Area]) -> Option<Area> {
    monitors
        .iter()
        .map(|monitor| (monitor.overlap(rect), *monitor))
        .filter(|(overlap, _)| *overlap > 0.0)
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, monitor)| monitor)
}

/// Fits `rect` to `monitor` so the window can be reached and used.
///
/// Two independent corrections, in this order:
///
/// 1. **Size.** A window larger than the monitor is shrunk to it. Restoring the
///    saved size would put the title bar, and part of the content, off-screen
///    with no way to drag it back.
/// 2. **Position.** A rect not meaningfully on the monitor is moved inside it,
///    keeping its offset where it still fits.
///
/// Size is corrected first because it changes the rect that position is then
/// judged against -- clamping a 4000px-wide window into a 1920 monitor and then
/// only checking its top-left corner would leave the right edge hanging off.
pub fn clamp(rect: &WindowState, monitor: &Area) -> WindowState {
    let mut fitted = *rect;

    if fitted.width > monitor.width {
        fitted.width = monitor.width;
    }
    if fitted.height > monitor.height {
        fitted.height = monitor.height;
    }

    // Below the minimum, rather than zero: a stored value too small to be a real
    // window is raised to something usable rather than refused, because refusing
    // would leave the user with whatever the previous state was.
    if fitted.width < MIN_WIDTH {
        fitted.width = MIN_WIDTH.min(monitor.width);
    }
    if fitted.height < MIN_HEIGHT {
        fitted.height = MIN_HEIGHT.min(monitor.height);
    }

    if !monitor.holds(&fitted) {
        // Centre on the monitor. An x/y that is merely slightly off is not worth
        // correcting -- the window is still usable -- so this only runs for a rect
        // that is genuinely off the monitor.
        fitted.x = monitor.x + (monitor.width - fitted.width) / 2.0;
        fitted.y = monitor.y + (monitor.height - fitted.height) / 2.0;
    }

    fitted
}

/// The live geometry of a window, as it should be stored.
///
/// A maximized or minimized window's *reported* geometry is not the user's
/// choice -- maximized reports the screen and minimized reports nothing useful --
/// so the caller's last normal rect is returned instead, keeping both flags
/// from being recorded as if they were sizes the user picked.
fn normal_rect(window: &WebviewWindow) -> Option<WindowState> {
    if window.is_maximized().unwrap_or(false) || window.is_minimized().unwrap_or(false) {
        return None;
    }
    let scale = window.scale_factor().ok()?;
    let position = window.outer_position().ok()?;
    let size = window.outer_size().ok()?;
    Some(WindowState {
        x: position.x as f64 / scale,
        y: position.y as f64 / scale,
        width: size.width as f64 / scale,
        height: size.height as f64 / scale,
        maximized: false,
    })
}

/// The state a fresh window -- or one with no stored geometry -- opens at.
fn configured_default(window: &WebviewWindow) -> Option<WindowState> {
    let scale = window.scale_factor().ok()?;
    let size = window.inner_size().ok()?;
    Some(WindowState {
        x: f64::NAN, // Resolved by the caller via `center()`.
        y: f64::NAN,
        width: size.width as f64 / scale,
        height: size.height as f64 / scale,
        maximized: false,
    })
}

/// Writes the window's current geometry, keeping the previous normal rect.
///
/// Called on the debounce timer and on exit. The previous rect is passed in
/// because a maximized window cannot report its own, and dropping the last
/// normal size would mean un-maximizing into fullscreen.
pub fn persist(window: &WebviewWindow, database: &Database, previous: &WindowState) {
    let maximized = window.is_maximized().unwrap_or(false);
    // Keep the size the user chose; only update the position when we can see a
    // real one. This is what makes a maximize-then-quit round trip lossless.
    let state = match normal_rect(window) {
        Some(rect) => rect,
        None => WindowState {
            x: previous.x,
            y: previous.y,
            width: previous.width,
            height: previous.height,
            maximized,
        },
    };
    if !state.is_usable() {
        return;
    }
    if let Err(error) = database.set_setting(WINDOW_KEY, &state) {
        tracing::warn!("could not store window state: {error}");
    }
}

/// Applies saved geometry to the window and shows it.
///
/// Called once, from `setup`, before the window has ever been visible. Every
/// path ends in `show()`, including the failure paths: a window that cannot be
/// restored should still open at the configured default rather than stay
/// invisible.
pub fn restore(window: &WebviewWindow, database: &Database) -> Option<WindowState> {
    let stored = database.setting::<WindowState>(WINDOW_KEY);

    let monitors: Vec<Area> = window
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(Area::from_monitor)
        .collect();

    // Whether the rect has a usable position at all. Read *before* clamping,
    // because `clamp` replaces the `NaN` sentinel with a real centered
    // position and the flag would then always come out false.
    let mut center_it = true;
    let chosen = match stored.filter(|state| state.is_usable()) {
        // Only reuse a stored position on a monitor that still exists. Picking
        // by overlap means undocking a laptop and relaunching finds no monitor
        // holding the old rect, and falls through to centering below instead of
        // restoring onto a display that is no longer attached.
        Some(state) => match target_monitor(&state, &monitors) {
            Some(monitor) => {
                center_it = false;
                Some(clamp(&state, &monitor))
            }
            None => configured_default(window)
                .map(|default| clamp(&default, &default_monitor(window, &monitors))),
        },
        None => configured_default(window)
            .map(|default| clamp(&default, &default_monitor(window, &monitors))),
    };

    let Some(state) = chosen else {
        // Nothing could be worked out. Show the window as configured rather than
        // leaving it hidden; the geometry is then wrong but reachable.
        let _ = window.show();
        return None;
    };

    let _ = window.set_size(LogicalSize::new(state.width, state.height));
    if center_it {
        let _ = window.center();
    } else {
        let _ = window.set_position(LogicalPosition::new(state.x, state.y));
    }

    if state.maximized {
        let _ = window.maximize();
    }

    // `unminimize` before `show`: a window left minimized by the last session
    // would otherwise be restored to a minimized rect and appear to open empty.
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();

    Some(state)
}

/// The monitor to fall back to when nothing is stored.
///
/// The window's own current monitor, or the primary one. Using `available[0]`
/// alone would put the window on whichever monitor happens to sort first, which
/// is not the one the user was last looking at.
fn default_monitor(window: &WebviewWindow, monitors: &[Area]) -> Area {
    window
        .current_monitor()
        .ok()
        .flatten()
        .map(|monitor| Area::from_monitor(&monitor))
        .or_else(|| monitors.first().copied())
        .unwrap_or(Area {
            // A plausible fallback so `clamp` has something to compare against
            // on a machine that reports no monitors at all.
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        })
}

/// Debounces geometry writes behind a generation counter.
///
/// `Moved`/`Resized` fire once per frame of a drag. Rather than holding a
/// timer per event, each event bumps a counter and spawns a task that sleeps
/// and then writes only if no newer event arrived in the meantime. The
/// comparison is what makes it a debounce rather than a delay: the last event
/// wins, and every earlier one becomes a no-op.
#[derive(Clone, Default)]
struct Debouncer {
    generation: Arc<AtomicU64>,
}

impl Debouncer {
    /// Records that geometry changed and arranges a write after the quiet period.
    ///
    /// Takes the `AppHandle` by value: the spawned task outlives this call, and
    /// a borrowed handle would not survive being moved into it.
    fn schedule(&self, app: AppHandle, database: Arc<Database>) {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let expected = generation;
        let debouncer = self.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(PERSIST_DEBOUNCE_MS)).await;
            // Superseded by a later event; that one owns the write.
            if debouncer.generation.load(Ordering::SeqCst) != expected {
                return;
            }
            let Some(window) = app.get_webview_window(WINDOW_LABEL) else {
                return;
            };
            let previous = database
                .setting::<WindowState>(WINDOW_KEY)
                .unwrap_or_default();
            persist(&window, &database, &previous);
        });
    }
}

/// Starts tracking the window's geometry, writing it as the user moves it.
///
/// Installs the listeners that call back into [`persist`], and returns the last
/// known normal rect so a caller can hand it to a final write.
pub fn attach(window: &WebviewWindow, database: Arc<Database>) {
    let debouncer = Debouncer::default();
    let app = window.app_handle().clone();
    // The listener must *own* the window rather than borrow it. `on_window_event`
    // hands the callback a `&Window` whose lifetime does not reach this
    // function's argument, so capturing `window` by reference would let the
    // borrow escape.
    let tracked = window.clone();

    let on_change = {
        let debouncer = debouncer.clone();
        let app = app.clone();
        let database = Arc::clone(&database);
        move || debouncer.schedule(app.clone(), Arc::clone(&database))
    };

    // Shared by the close and destroy arms, which both need the previous rect.
    let write_geometry = {
        let database = Arc::clone(&database);
        let tracked = tracked.clone();
        move || {
            let previous = database
                .setting::<WindowState>(WINDOW_KEY)
                .unwrap_or_default();
            persist(&tracked, &database, &previous);
        }
    };
    let on_destroy = write_geometry.clone();

    window.on_window_event(move |event| match event {
        WindowEvent::Moved(_) | WindowEvent::Resized(_) => on_change(),
        // The close button hides to the tray rather than exiting, so this is the
        // last chance to record where the window was left.
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            write_geometry();
            let _ = tracked.hide();
        }
        WindowEvent::Destroyed => {
            // The debounced write may still be pending, and a destroyed window
            // reports no geometry -- so flush the pending intent now.
            on_change();
            on_destroy();
        }
        _ => {}
    });
}

/// Writes the final geometry as the app exits.
///
/// Kept separate from [`attach`] because exit has no window event to hang off:
/// the tray's Quit path and `RunEvent::Exit` both bypass `CloseRequested`.
pub fn persist_on_exit(app: &AppHandle, database: &Database) {
    let Some(window) = app.get_webview_window(WINDOW_LABEL) else {
        return;
    };
    let previous = database
        .setting::<WindowState>(WINDOW_KEY)
        .unwrap_or_default();
    persist(&window, database, &previous);
}
