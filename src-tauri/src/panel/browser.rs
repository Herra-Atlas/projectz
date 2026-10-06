//! The right panel's browser: a child webview inside the app's own window.
//!
//! **Not a model tool.** For the same reason [`super::fs`] is not one: the browser
//! is app chrome the *user* drives. It has no `ToolSpec`, never passes through the
//! permission gate, and is never advertised in a request's `tools` array. A future
//! `browser_read` tool would live in `ai/tools/` and *drive this webview* rather
//! than replace it, so the user watches what the agent is looking at.
//!
//! # Why a child webview rather than an iframe
//!
//! An `<iframe>` inside the panel cannot load most of the web -- carriers send
//! `X-Frame-Options` and `frame-ancestors` -- and an address bar cannot drive it
//! cross-origin. A real child webview is a genuine browsing surface with its own
//! cookie jar and its own JavaScript context.
//!
//! That separation is also the best safety property this feature has, and it comes
//! from the architecture rather than from a filter: **the browser is logged out of
//! everything the user is logged into.** Their Google session, their bank, their
//! GitHub are all invisible to it.
//!
//! # Why it is created from Rust rather than from JavaScript
//!
//! `@tauri-apps/api`'s `Webview` constructor needs the same `unstable` Cargo
//! feature *and* grants the frontend a standing capability to create and move
//! webviews: `core:webview:allow-create-webview`, `...allow-set-webview-size`,
//! `...allow-set-webview-position`, `...allow-webview-close`. Building it here
//! means the frontend can only ask for the specific operations below, so nothing
//! in a chat transcript can spawn or reposition a browsing surface.
//!
//! # Why the webview is destroyed rather than hidden
//!
//! Hiding a webview leaves its document, its JavaScript heap, its render tree and
//! the page's own timers alive. A "closed" browser would go on costing memory and
//! battery for the rest of the session, so the panel closes it outright and
//! reopening starts a fresh load. Nothing is restored: re-navigating to the page
//! you just left is itself a load nobody asked for. See `BrowserTab.tsx` for the
//! frontend half.

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex, MutexGuard,
};

use serde::Serialize;
use tauri::{
    webview::{NewWindowResponse, PageLoadEvent, PageLoadPayload, WebviewBuilder},
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Runtime, Url, Webview, WebviewUrl,
};

/// The webview's label. Fixed rather than generated, because there is only ever
/// one browser in the panel and a stable label makes "is it already open?" a
/// lookup rather than a scan.
pub const BROWSER_LABEL: &str = "projectz-panel-browser";

/// Event carrying what the browser is currently showing.
pub const BROWSER_PAGE_EVENT: &str = "panel-browser-page";

/// The engine used when the address bar is given something that is not a URL.
///
/// One fixed engine rather than "let the user's default browser decide", because
/// the search and the browser must not be different systems: a query typed into
/// the panel should be answered by the same engine `search_web` uses, or the two
/// features disagree about what the web says about a query.
pub const SEARCH_ENGINE: &str = "https://duckduckgo.com/html/?q=";

/// One entry in the browser's tab strip.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserTabSummary {
    pub id: u64,
    /// The tab's current page, or empty before its first navigation.
    pub url: String,
}

/// What the browser is currently showing.
///
/// **No title.** A child webview has no way to report one without an IPC channel
/// the page could block, and a title bar is not part of this view: the address is
/// what identifies a page here, and a half-working title is worse than none.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserState {
    /// The page currently loaded, or empty before the first navigation.
    pub url: String,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    /// True between a navigation starting and the page finishing.
    pub loading: bool,
    /// Every open tab, oldest first. The strip renders from this alone.
    #[serde(default)]
    pub tabs: Vec<BrowserTabSummary>,
    /// The tab on screen, if any.
    #[serde(default)]
    pub active_tab: Option<u64>,
}

/// What the browser remembers between navigations.
///
/// **History is ours, not the webview's.** Tauri exposes no `go_back` or
/// `go_forward` on a child webview, so the back button walks a list this module
/// keeps. The difference from a real browser's session history is deliberate and
/// worth stating: a page reached through a redirect the panel never saw is not in
/// this list, so `back` skips over it.
#[derive(Default)]
struct History {
    /// Which tab this history belongs to. Set once at creation; the strip keys
    /// on it, so two tabs never share an id.
    id: u64,
    /// Visited pages, oldest first.
    entries: Vec<String>,
    /// Where in `entries` the current page sits.
    index: Option<usize>,
    loading: bool,
}

impl History {
    /// Names a page before it has loaded, so a tab opened from a link shows its
    /// address while the load is in flight.
    ///
    /// A seed rather than a push: `on_page_load` pushes the page when it lands,
    /// and seeding through `push` would record it twice -- once now and once on
    /// arrival, breaking back/forward from the first press.
    fn seed(&mut self, url: &str) {
        if self.entries.is_empty() {
            self.entries.push(url.to_string());
        }
    }
    /// Records a completed navigation.
    ///
    /// **A page already in the list moves the index rather than adding an entry.**
    /// This is the back and forward buttons' own navigation arriving back here:
    /// `step` has already moved the index to the entry being returned to, and if
    /// this then treated it as a new visit it would truncate the list to that
    /// entry and throw away everything ahead of it -- so pressing back would
    /// destroy the forward history and `forward` would stop working for the rest
    /// of the session.
    ///
    /// Anything genuinely new after a step truncates as well, because the user has
    /// moved on from what was ahead: keeping a forward stack past a fresh
    /// navigation is how "forward" ends up replaying a page the user has already
    /// deliberately left behind.
    fn push(&mut self, url: &str) {
        if let Some(existing) = self.entries.iter().position(|entry| entry == url) {
            self.index = Some(existing);
            return;
        }
        if let Some(index) = self.index {
            self.entries.truncate(index + 1);
        }
        self.entries.push(url.to_string());
        self.index = Some(self.entries.len() - 1);
    }

    fn can_go_back(&self) -> bool {
        self.index.is_some_and(|index| index > 0)
    }

    fn can_go_forward(&self) -> bool {
        self.index
            .is_some_and(|index| index + 1 < self.entries.len())
    }

    /// The url `state` reports, which is the entry the current page sits on.
    fn current(&self) -> String {
        self.index
            .and_then(|index| self.entries.get(index))
            .cloned()
            .unwrap_or_default()
    }

    /// Steps one entry back, returning the page now current.
    fn step_back(&mut self) -> Option<String> {
        let index = self.index?.checked_sub(1)?;
        self.index = Some(index);
        Some(self.entries[index].clone())
    }

    /// Steps one entry forward, returning the page now current.
    fn step_forward(&mut self) -> Option<String> {
        let index = self.index?.checked_add(1)?;
        // Bound-checked because `index + 1` can leave the list, and reading past
        // its end would panic rather than report "there is nothing forward".
        let url = self.entries.get(index)?.clone();
        self.index = Some(index);
        Some(url)
    }
}

/// Managed so the browser's tabs outlive a single command call.
///
/// **Tabs live here, not in the frontend.** The child webview is one surface and
/// tab switches are navigations of it, so the list has to be where the commands
/// already are. The frontend renders `BrowserState.tabs` and asks to switch; it
/// never invents a tab on its own, or the two would disagree about which page is
/// on screen.
#[derive(Default)]
pub struct Browser {
    tabs: Mutex<Tabs>,
    generation: AtomicU64,
}

/// Every open tab and which one is on screen.
///
/// One webview, many histories: switching tabs navigates the single surface to
/// the tab's current page, so there is still only ever one page alive. A tab
/// that never navigated holds an empty history and draws the empty state.
#[derive(Default)]
struct Tabs {
    entries: Vec<History>,
    active: Option<usize>,
    next_id: u64,
}

impl Tabs {
    /// The history on screen, if any tab is open.
    fn current(&mut self) -> Option<&mut History> {
        let index = self.active?;
        self.entries.get_mut(index)
    }

    /// Opens a tab and brings it forward, returning its id.
    fn open(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.entries.push(History {
            id,
            ..History::default()
        });
        self.active = Some(self.entries.len() - 1);
        id
    }

    /// Opens a tab on a page that already resolved, for `_blank` links.
    ///
    /// The entry is seeded rather than pushed through the usual load path,
    /// because the page has not loaded yet -- `on_page_load` records it when it
    /// does, and seeding here is what shows the address meanwhile.
    fn open_on(&mut self, url: &str) -> u64 {
        let id = self.open();
        if let Some(current) = self.current() {
            current.seed(url);
        }
        id
    }

    /// Brings a tab forward by id. Unknown ids are ignored, because a click and
    /// a close can race and refusing would leave the strip pointing nowhere.
    fn select(&mut self, id: u64) {
        if let Some(index) = self.entries.iter().position(|entry| entry.id == id) {
            self.active = Some(index);
        }
    }

    /// Closes a tab, returning the page now on screen, if any.
    ///
    /// The neighbour to the right takes over, or the new last entry when the end
    /// closed -- the same rule every tab strip follows. Closing the last tab
    /// leaves no active tab rather than a blank one, so the empty state shows.
    fn close(&mut self, id: u64) -> Option<String> {
        let index = self.entries.iter().position(|entry| entry.id == id)?;
        self.entries.remove(index);
        if self.entries.is_empty() {
            self.active = None;
            return None;
        }
        let next = index.min(self.entries.len() - 1);
        self.active = Some(next);
        Some(self.entries[next].current())
    }

    fn summaries(&self) -> Vec<BrowserTabSummary> {
        self.entries
            .iter()
            .map(|entry| BrowserTabSummary {
                id: entry.id,
                url: entry.current(),
            })
            .collect()
    }

    fn active_id(&self) -> Option<u64> {
        self.active
            .and_then(|index| self.entries.get(index))
            .map(|entry| entry.id)
    }

    /// The full state the frontend renders from. One builder rather than each
    /// command assembling it, so a new field cannot be forgotten on one path.
    fn state(&self) -> BrowserState {
        let current = self.active.and_then(|index| self.entries.get(index));
        BrowserState {
            url: current.map(|entry| entry.current()).unwrap_or_default(),
            can_go_back: current.is_some_and(|entry| entry.can_go_back()),
            can_go_forward: current.is_some_and(|entry| entry.can_go_forward()),
            loading: current.is_some_and(|entry| entry.loading),
            tabs: self.summaries(),
            active_tab: self.active_id(),
        }
    }
}

/// Locks the browser's tabs, or reports that the lock is poisoned.
///
/// Its own function because every command needs it and writing `.lock()` out in
/// each is how one of them ends up handling a poisoned lock differently.
fn tabs_of(browser: &Browser) -> Result<MutexGuard<'_, Tabs>, String> {
    browser
        .tabs
        .lock()
        .map_err(|_| "The browser's tabs are unavailable".to_string())
}

/// Turns whatever was typed in the address bar into a URL to load.
///
/// **A phrase is a search, anything URL-shaped is a URL.** One field for both is
/// what a browser address bar is, and guessing wrong in the other direction --
/// searching for `example.com/foo` instead of navigating to it -- is the failure
/// that makes an address bar feel broken.
///
/// A scheme is recognised explicitly rather than by the presence of a dot: a
/// search query can perfectly well contain one, and `localhost:8000` has none but
/// is plainly not a search for the words "localhost".
pub fn resolve_input(input: &str) -> Result<Url, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Enter an address to open".into());
    }

    // A real scheme is recognised *before* the parse below, and only when it is
    // `http`, `https`, or is written the way a browser expects a scheme to be --
    // `scheme://`.
    //
    // The reason for not trusting `Url::parse` on its own is `localhost:5173`,
    // which parses successfully as the custom scheme `localhost` with the path
    // `5173`. It would then be rejected as an unknown scheme, so typing a dev
    // server's address into the panel would say "only http and https are
    // allowed" about the most obvious thing anyone would type here.
    if has_a_browser_scheme(trimmed) {
        let url = Url::parse(trimmed).expect("the scheme was just checked");
        return check_scheme(url);
    }

    // No usable scheme. `localhost:8000` is the case that matters here, and the
    // host detection below is what turns it into `http://localhost:8000`.
    if looks_like_a_host(trimmed) {
        return Url::parse(&format!("http://{trimmed}"))
            .map_err(|error| format!("That address could not be opened: {error}"));
    }

    // Not a URL at all, so it is a query.
    Url::parse(&format!("{SEARCH_ENGINE}{}", urlencode(trimmed)))
        .map_err(|error| format!("That search could not be run: {error}"))
}

/// Whether the input opens with a scheme a browser would honour.
///
/// `scheme://` counts for anything, so `ftp://host` is recognised as an address
/// and then refused by [`check_scheme`] with a message that names the problem --
/// rather than being searched for as a phrase, which would look like the panel
/// did not understand it.
fn has_a_browser_scheme(input: &str) -> bool {
    if input.starts_with("http://") || input.starts_with("https://") || input.starts_with("file://")
    {
        return true;
    }
    // `javascript:` and `data:` have no `//`, and they are exactly the inputs this
    // check exists to stop. Recognised by their colon and refused by name, so the
    // user is told the scheme is not allowed rather than being handed a web
    // search for the text "javascript:alert(1)".
    let Some((scheme, rest)) = input.split_once(':') else {
        return false;
    };
    if !is_a_scheme(scheme) {
        return false;
    }
    // `javascript:alert(1)` and `data:text/html,...` carry their payload straight
    // after the colon with no slash, and they are exactly the inputs this check
    // exists to stop. Refused by name so the user is told the scheme is not
    // allowed, rather than being handed a web search for the text of the payload.
    const REFUSED_WITHOUT_SLASH: [&str; 2] = ["javascript", "data"];
    if REFUSED_WITHOUT_SLASH
        .iter()
        .any(|refused| scheme.eq_ignore_ascii_case(refused))
    {
        return true;
    }
    // Otherwise a scheme has to be spelled out and followed by a slash.
    // `localhost:5173` is shaped exactly like a scheme plus a path -- the word
    // `localhost` is a legal scheme and `5173` a legal opaque path -- so the shape
    // alone is not enough, and requiring the slash is what keeps the most obvious
    // thing anyone would type into a dev-server panel working.
    !rest.is_empty() && rest.starts_with('/')
}

/// Whether a string is shaped like a URL scheme.
///
/// The rules the URL standard sets out, kept short deliberately: this decides
/// whether the rest of the input is an address, so accepting something odd here
/// means a phrase starting with a colon can be mistaken for one.
fn is_a_scheme(scheme: &str) -> bool {
    let mut characters = scheme.chars();
    // A scheme must start with a letter. This is also what excludes a Windows
    // drive letter, which is a single character and therefore not a scheme.
    characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
        })
}

/// Whether a string names a host rather than being a phrase.
///
/// Deliberately conservative about the dot: a bare `hello.world` is two plausible
/// words and one implausible TLD, so it is treated as a search. A user who meant
/// the site can type `https://` and get it either way.
fn looks_like_a_host(input: &str) -> bool {
    let (host, _) = input.split_once('/').unwrap_or((input, ""));
    // A host cannot contain whitespace, and this is what stops a *sentence*
    // reaching the host test at all. Checked here rather than left to the URL
    // parser, because a parser's rejection would come back as "that address
    // could not be opened" -- an error about the wrong thing, when the right
    // answer was to search for what the user typed.
    if host.is_empty() || host.chars().any(char::is_whitespace) {
        return false;
    }
    if host.starts_with('[') || host.contains(':') {
        return true;
    }
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2 {
        return host.eq_ignore_ascii_case("localhost");
    }
    // A plausible TLD rather than a sentence, so `what is this.error` is a search
    // and `example.com` is a site. A single character is never a TLD.
    labels.last().is_some_and(|last| {
        last.len() >= 2
            && last
                .chars()
                .all(|character| character.is_ascii_alphabetic())
    })
}

/// Refuses a scheme this browser will not load.
///
/// **The reason this app can afford to be strict.** This process holds the user's
/// files and its agent's tools, so a scheme that reads the disk or launches a
/// handler is a way out of the sandbox the whole permission design assumes.
/// `http` and `https` are always allowed; `file` is allowed only when it names a
/// file inside the open workspace, so a local page can be previewed without the
/// panel becoming a reader for the whole disk.
fn check_scheme(url: Url) -> Result<Url, String> {
    match url.scheme() {
        "http" | "https" => Ok(url),
        "file" if file_is_within_workspace(&url) => Ok(url),
        "file" => Err(
            "Only files inside the open workspace can be opened here.".to_string(),
        ),
        other => Err(format!(
            "`{other}` pages cannot be opened here. Only http, https, and workspace files are allowed."
        )),
    }
}

/// Whether a `file:` URL names something inside the open workspace.
///
/// The workspace root is stored canonicalized, so the candidate is canonicalized
/// too before comparing -- otherwise a `..` segment or a symlinked prefix would
/// make an outside path read as inside, or vice versa. A path that cannot be
/// canonicalized (missing file, bad encoding) is not inside.
fn file_is_within_workspace(url: &Url) -> bool {
    let Ok(path) = url.to_file_path() else {
        return false;
    };
    let Some(root) = crate::ai::tools::workspace::root() else {
        return false;
    };
    let Ok(candidate) = path.canonicalize() else {
        return false;
    };
    candidate.starts_with(root)
}

/// Whether a webview navigation may proceed.
///
/// The same boundary as [`check_scheme`], minus the workspace check: by the time
/// a loaded page navigates itself, its URL is already known-good, so a `file:`
/// page following a relative link stays allowed without re-resolving the root on
/// every hop. Anything else is refused.
fn scheme_is_allowed(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https" | "file")
}

/// Percent-encodes a search phrase for a query string.
fn urlencode(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char)
            }
            b' ' => encoded.push('+'),
            other => encoded.push_str(&format!("%{other:02X}")),
        }
    }
    encoded
}

/// Opens a page, creating the webview if this is the first one.
///
/// A tab is opened for the first navigation and reused afterwards: the address
/// bar navigates the current tab, while links that asked for a new window arrive
/// through `panel_browser_open_tab` instead.
#[tauri::command]
pub async fn panel_browser_navigate(app: AppHandle, url: String) -> Result<BrowserState, String> {
    let target = resolve_input(&url)?;
    if let Some(webview) = app.get_webview(BROWSER_LABEL) {
        navigate(&webview, target)?;
        let state = current_state(&app);
        emit_state(&app, &state);
        return Ok(state);
    }
    create_browser(&app, target)
}

/// Opens a fresh tab and navigates it to the given page.
///
/// The caller is a `_blank` link (via the new-window event) or the `+` button
/// (with no URL, drawing the empty state). The new tab takes focus; the page it
/// came from is untouched.
#[tauri::command]
pub async fn panel_browser_open_tab(
    app: AppHandle,
    url: Option<String>,
) -> Result<BrowserState, String> {
    let target = match url {
        Some(input) => Some(resolve_input(&input)?),
        None => None,
    };
    let Some(webview) = app.get_webview(BROWSER_LABEL) else {
        // No surface yet. With a URL this is the first navigation, so create the
        // browser onto it; without one there is nothing to show, so just record
        // the empty tab.
        if let Some(target) = target {
            return create_browser(&app, target);
        }
        if let Some(browser) = app.try_state::<Browser>() {
            if let Ok(mut tabs) = tabs_of(&browser) {
                tabs.open();
                let state = tabs.state();
                emit_state(&app, &state);
                return Ok(state);
            }
        }
        return Ok(BrowserState::default());
    };
    let state = {
        let Some(browser) = app.try_state::<Browser>() else {
            return Err("The browser is not available".to_string());
        };
        let mut tabs = tabs_of(&browser)?;
        match &target {
            Some(target) => {
                tabs.open_on(target.as_str());
            }
            None => {
                tabs.open();
            }
        }
        tabs.state()
    };
    match &target {
        Some(target) => {
            navigate(&webview, target.clone())?;
        }
        None => {
            // An empty tab shows the DOM empty state ("Open a page"), so the
            // surface is hidden rather than left painted with the previous tab.
            let _ = webview.hide();
        }
    }
    emit_state(&app, &state);
    Ok(state)
}

/// Switches to a tab, navigating the single surface to its page.
///
/// A tab that never navigated hides the surface so the DOM empty state shows,
/// rather than replaying a stale address or leaving the previous page painted.
#[tauri::command]
pub async fn panel_browser_select_tab(app: AppHandle, id: u64) -> Result<BrowserState, String> {
    let Some(webview) = app.get_webview(BROWSER_LABEL) else {
        return Err("The browser is not open".to_string());
    };
    let (target, state) = {
        let Some(browser) = app.try_state::<Browser>() else {
            return Err("The browser is not available".to_string());
        };
        let mut tabs = tabs_of(&browser)?;
        tabs.select(id);
        let current = tabs
            .current()
            .map(|entry| entry.current())
            .unwrap_or_default();
        let state = tabs.state();
        (current, state)
    };
    if target.is_empty() {
        // An empty tab draws the DOM empty state, so the surface is hidden
        // rather than left painted with the previous tab's page.
        let _ = webview.hide();
    } else {
        match Url::parse(&target) {
            Ok(url) => {
                let _ = navigate(&webview, url);
            }
            Err(error) => {
                tracing::debug!(url = %target, error = %error, "browser tab entry is not a url")
            }
        }
    }
    emit_state(&app, &state);
    Ok(state)
}

/// Closes a tab, showing the neighbour that takes its place.
///
/// Closing the last tab destroys the webview too: an empty browser costs a
/// surface for nothing, and reopening starts clean.
#[tauri::command]
pub async fn panel_browser_close_tab(app: AppHandle, id: u64) -> Result<BrowserState, String> {
    let (next, state) = {
        let Some(browser) = app.try_state::<Browser>() else {
            return Err("The browser is not available".to_string());
        };
        let mut tabs = tabs_of(&browser)?;
        if tabs.entries.len() == 1 && tabs.entries.iter().any(|entry| entry.id == id) {
            browser.generation.fetch_add(1, Ordering::SeqCst);
        }
        let next = tabs.close(id);
        let state = tabs.state();
        (next, state)
    };
    let Some(webview) = app.get_webview(BROWSER_LABEL) else {
        return Ok(state);
    };
    match next {
        Some(target) if !target.is_empty() => match Url::parse(&target) {
            Ok(url) => {
                let _ = navigate(&webview, url);
            }
            Err(error) => {
                tracing::debug!(url = %target, error = %error, "browser tab entry is not a url")
            }
        },
        // A neighbour tab that never navigated draws the DOM empty state, so
        // hide the surface rather than leaving the closed tab's page painted.
        Some(_) => {
            let _ = webview.hide();
        }
        _ => {
            let _ = webview.close();
        }
    }
    emit_state(&app, &state);
    Ok(state)
}

async fn open_link_tab(app: AppHandle, input: String, generation: u64) -> Result<(), String> {
    let target = resolve_input(&input)?;
    let Some(webview) = app.get_webview(BROWSER_LABEL) else {
        return Ok(());
    };
    {
        let Some(browser) = app.try_state::<Browser>() else {
            return Ok(());
        };
        if browser.generation.load(Ordering::SeqCst) != generation {
            return Ok(());
        }
        let mut tabs = tabs_of(&browser)?;
        if tabs.entries.is_empty() {
            return Ok(());
        }
        tabs.open_on(target.as_str());
        emit_state(&app, &tabs.state());
    }
    navigate(&webview, target)
}

/// Creates the webview onto a page, opening the first tab for it.
///
/// Used when the address bar or an explicit browser request starts the first
/// page, before a browser surface exists.
fn create_browser(app: &AppHandle, target: Url) -> Result<BrowserState, String> {
    // `get_window` rather than `get_webview_window`: `add_child` hangs a webview off
    // the window's own client area, and only the `Window` exposes it.
    let window = app
        .get_window("main")
        .ok_or_else(|| "The main window is not available".to_string())?;

    let opener = app.clone();
    let link_generation = app
        .try_state::<Browser>()
        .map(|browser| browser.generation.load(Ordering::SeqCst))
        .unwrap_or_default();
    // Off-screen at 1x1 until the frontend reports the container's real geometry.
    // A webview created at 0,0 with a guessed size would be visible for the frame
    // between creation and the first measurement, sitting over the chat.
    let builder = WebviewBuilder::new(BROWSER_LABEL, WebviewUrl::External(target.clone()))
        // Zoom keys, because a page that cannot be zoomed is unreadable for some
        // people and it costs one line. This webview is a document viewer rather
        // than a general-purpose browser, so nothing else from a browser's chrome
        // is enabled.
        .zoom_hotkeys_enabled(true)
        // http(s) always, plus workspace files -- enforced again here as well as
        // in `resolve_input`: a page can navigate itself to another scheme, and
        // this is the check that stops that.
        .on_navigation(scheme_is_allowed)
        // A `_blank` link becomes a tab, not a window. Denied rather than
        // created: there is exactly one surface in this panel, and a second one
        // would need its own geometry, its own bounds reporting, and its own
        // teardown -- for a popup nobody asked to keep.
        .on_new_window(move |url, _features| {
            // Refused schemes never become tabs. A `javascript:` URL handed to a
            // tab would navigate the one surface to it, which is worse than the
            // click doing nothing.
            if !scheme_is_allowed(&url) {
                return NewWindowResponse::Deny;
            }
            let target = url.to_string();
            let app = opener.clone();
            tauri::async_runtime::spawn(async move {
                let _ = open_link_tab(app, target, link_generation).await;
            });
            NewWindowResponse::Deny
        })
        .on_page_load(on_page_load);

    window
        .add_child(
            builder,
            LogicalPosition::new(0.0, 0.0),
            LogicalSize::new(1.0, 1.0),
        )
        .map_err(|error| format!("The browser view could not be created: {error}"))?;

    // Recorded here rather than left to `on_page_load`, because that can fire
    // before the frontend has registered its listener -- and the toolbar would
    // then show no address for a page that did load.
    if let Some(browser) = app.try_state::<Browser>() {
        if let Ok(mut tabs) = tabs_of(&browser) {
            // A first tab when none exists, the current tab otherwise: creating
            // the surface never invents navigation of its own.
            if tabs.active.is_none() {
                tabs.open_on(target.as_str());
            } else if let Some(current) = tabs.current() {
                current.push(target.as_str());
            }
            let state = tabs.state();
            emit_state(app, &state);
            return Ok(state);
        }
    }

    Ok(BrowserState::default())
}

/// Reads the current state without emitting it, for command return values.
///
/// Commands return the state directly so the caller renders immediately; the
/// event still fires for every other listener. Both carry the same object built
/// by the same function, so they cannot disagree.
fn current_state<R: Runtime>(app: &AppHandle<R>) -> BrowserState {
    app.try_state::<Browser>()
        .and_then(|browser| tabs_of(&browser).ok().map(|tabs| tabs.state()))
        .unwrap_or_default()
}

fn emit_state<R: Runtime>(app: &AppHandle<R>, state: &BrowserState) {
    let _ = app.emit_to("main", BROWSER_PAGE_EVENT, state.clone());
}

/// Sends the frontend what the browser is now showing.
fn on_page_load<R: Runtime>(webview: Webview<R>, payload: PageLoadPayload<'_>) {
    let app = webview.app_handle().clone();
    let started = payload.event() == PageLoadEvent::Started;

    if started {
        // Only the loading flag here: the url is reported when the page
        // finishes, so a redirect chain does not print an intermediate address
        // that was never the page the reader ended up on.
        if let Some(browser) = app.try_state::<Browser>() {
            if let Ok(mut tabs) = tabs_of(&browser) {
                if let Some(current) = tabs.current() {
                    current.loading = true;
                }
                let state = tabs.state();
                emit_state(&app, &state);
            }
        }
        return;
    }

    if payload.event() != PageLoadEvent::Finished {
        return;
    }

    let url = payload.url().to_string();

    if url == "about:blank" {
        // Parking navigations (`about:blank` on empty tabs) are not history.
        // Recording them would put a blank entry behind every new tab's first
        // real page, so back would go to nothing.
        if let Some(browser) = app.try_state::<Browser>() {
            if let Ok(tabs) = tabs_of(&browser) {
                emit_state(&app, &tabs.state());
            }
        }
        return;
    }

    let mut show = false;
    if let Some(browser) = app.try_state::<Browser>() {
        if let Ok(mut tabs) = tabs_of(&browser) {
            if let Some(current) = tabs.current() {
                current.push(&url);
                current.loading = false;
                show = true;
            }
            let state = tabs.state();
            emit_state(&app, &state);
        }
    }
    if show {
        let _ = webview.show();
    }
}

/// Navigates an existing webview, marking the load as started.
///
/// Separate from the command above because that one creates the webview and this
/// one only moves it, and the difference is a branch the caller can see.
///
/// **Async, and that is load-bearing rather than stylistic.** A synchronous
/// command runs on the main thread, and `Window::add_child` posts the build to
/// the main thread and then blocks on a channel waiting for it. Called from the
/// main thread that is a deadlock: the command never returns, the frontend's
/// promise never settles, and the browser sits on its empty state with no error
/// anywhere -- which is exactly what it did. Declaring the callers `async` moves
/// the command onto Tauri's async pool, so the main thread is free to run the
/// posted build. Every command that touches a webview is async for the same reason.
fn navigate<R: Runtime>(webview: &Webview<R>, target: Url) -> Result<(), String> {
    if let Some(browser) = webview.app_handle().try_state::<Browser>() {
        if let Ok(mut tabs) = tabs_of(&browser) {
            if let Some(current) = tabs.current() {
                current.loading = true;
            }
        }
    }
    // Keep the surface visible while navigating. Hiding it first can prevent a
    // child webview from completing a new-window navigation, leaving its tab blank.
    let _ = webview.show();
    webview
        .navigate(target)
        .map_err(|error| format!("That page could not be opened: {error}"))
}

/// Moves the browser to wherever its container now is.
///
/// Called on every resize rather than only on a settled width, because the
/// panel's drag writes its width straight to an inline style and commits to React
/// state only on release -- a webview watching state would lag the whole gesture.
/// `x` and `y` are the parent window's logical pixels, which is the space
/// `getBoundingClientRect` reports in.
///
/// Async for the same reason as [`panel_browser_navigate`]: `set_position` and
/// `set_size` dispatch to the main thread, and a synchronous command would be
/// occupying it.
#[tauri::command]
pub async fn panel_browser_set_bounds(app: AppHandle, x: f64, y: f64, width: f64, height: f64) {
    let Some(webview) = app.get_webview(BROWSER_LABEL) else {
        // Not open. The frontend mounts this view before asking for bounds, so
        // this is the ordinary "no browser yet" case rather than an error worth
        // reporting to a user.
        return;
    };
    if width < 1.0 || height < 1.0 {
        // A collapsed panel reports zero, and a zero-sized webview on Windows is
        // what makes one flash at 1x1 in the corner of the window. The existing
        // bounds are left alone instead, and the frontend hides it in the same
        // pass that reported this.
        return;
    }
    let _ = webview.set_position(LogicalPosition::new(x, y));
    let _ = webview.set_size(LogicalSize::new(width, height));
}

/// Shows or hides the browser without destroying it.
///
/// Used only for a tab switch or a collapse, so a webview that is merely not on
/// screen does not stay painted over the chat. The real teardown is
/// [`panel_browser_close`], and it is what frees the page's memory.
///
/// Async because `show` and `hide` dispatch to the main thread too.
#[tauri::command]
pub async fn panel_browser_set_visible(app: AppHandle, visible: bool) {
    let Some(webview) = app.get_webview(BROWSER_LABEL) else {
        return;
    };
    if visible {
        // An empty or loading tab must not resurface the previous page over the
        // DOM empty state or while its destination is still loading.
        if let Some(browser) = app.try_state::<Browser>() {
            if let Ok(mut tabs) = tabs_of(&browser) {
                if tabs
                    .current()
                    .is_some_and(|entry| entry.current().is_empty() || entry.loading)
                {
                    return;
                }
            }
        }
        let _ = webview.show();
    } else {
        let _ = webview.hide();
    }
}

/// Reloads the current page.
///
/// Async because `reload` dispatches to the main thread.
#[tauri::command]
pub async fn panel_browser_reload(app: AppHandle) -> Result<BrowserState, String> {
    let Some(webview) = app.get_webview(BROWSER_LABEL) else {
        return Ok(BrowserState::default());
    };
    if let Some(browser) = app.try_state::<Browser>() {
        if let Ok(mut tabs) = tabs_of(&browser) {
            if let Some(current) = tabs.current() {
                current.loading = true;
            }
            let state = tabs.state();
            emit_state(&app, &state);
        }
    }
    let _ = webview.reload();
    Ok(current_state(&app))
}

/// Goes back one page in the current tab's history.
///
/// Async because the navigation it performs dispatches to the main thread.
#[tauri::command]
pub async fn panel_browser_back(app: AppHandle) -> Result<BrowserState, String> {
    step(&app, Step::Back).await
}

/// Goes forward one page in the current tab's history.
///
/// Async for the same reason as [`panel_browser_back`].
#[tauri::command]
pub async fn panel_browser_forward(app: AppHandle) -> Result<BrowserState, String> {
    step(&app, Step::Forward).await
}

/// Which way through the history to move.
#[derive(Clone, Copy)]
enum Step {
    Back,
    Forward,
}

/// Moves one entry and navigates to it.
///
/// One function rather than two near-identical ones because the two differ only
/// in which method of [`History`] they call, and a pair of copies is a pair of
/// places for the "the step happens *before* the navigation, so `on_page_load` does
/// not push the page we are leaving" rule to be kept in agreement.
///
/// **The step moves the index, and that has to happen before navigating.** The
/// navigation fires `on_page_load`, which pushes the page it landed on. Stepping
/// after that would move the index onto the entry just visited, so `back` would
/// appear to do nothing and would need pressing twice to leave a page.
async fn step(app: &AppHandle, direction: Step) -> Result<BrowserState, String> {
    let Some(webview) = app.get_webview(BROWSER_LABEL) else {
        return Ok(BrowserState::default());
    };
    let target = {
        let Some(browser) = app.try_state::<Browser>() else {
            return Err("The browser is not available".to_string());
        };
        let Ok(mut tabs) = tabs_of(&browser) else {
            return Err("The browser's tabs are unavailable".to_string());
        };
        let Some(current) = tabs.current() else {
            return Ok(tabs.state());
        };
        match direction {
            Step::Back => current.step_back(),
            Step::Forward => current.step_forward(),
        }
    };
    let Some(target) = target else {
        // Nothing behind us, or nothing ahead. The button is disabled in the
        // toolbar for this, so reaching here means the two disagree -- which is
        // reported rather than acted on, because navigating nowhere is worse
        // than not navigating.
        return Ok(current_state(app));
    };
    match Url::parse(&target) {
        Ok(url) => {
            let _ = webview.navigate(url);
        }
        // Only reachable if an entry was stored that cannot be parsed, and
        // `push` only ever stores one that was already parsed by the URL crate.
        Err(error) => {
            tracing::debug!(url = %target, error = %error, "browser history entry is not a url")
        }
    }
    Ok(current_state(app))
}

/// Destroys the browser, freeing the page's memory.
///
/// **Tearing down rather than hiding is the point.** A hidden webview keeps its
/// document, its JavaScript heap, its render tree and the page's own timers alive,
/// so a "closed" browser would go on costing memory and battery for the rest of
/// the session. Nothing is restored on reopen: the webview is created empty and
/// waits to be told what to show, because re-navigating to the page you just left
/// is itself a load nobody asked for.
///
/// Async because `close` dispatches to the main thread -- and this is the one that
/// runs during a teardown, so a synchronous version would hold the main thread
/// busy destroying exactly the webview the frontend is waiting on.
#[tauri::command]
pub async fn panel_browser_close(app: AppHandle) {
    if let Some(browser) = app.try_state::<Browser>() {
        browser.generation.fetch_add(1, Ordering::SeqCst);
    }
    if let Some(webview) = app.get_webview(BROWSER_LABEL) {
        let _ = webview.close();
    }
    if let Some(browser) = app.try_state::<Browser>() {
        if let Ok(mut tabs) = tabs_of(&browser) {
            // Cleared alongside the webview so one created later does not report a
            // back button for a page that no longer exists.
            *tabs = Tabs::default();
        }
    }
}

/// Installs the managed state the commands read.
///
/// From `setup` rather than lazily on first use, so the very first
/// `panel_browser_open` finds a live [`Browser`].
pub fn install<R: Runtime>(app: &AppHandle<R>) {
    if app.try_state::<Browser>().is_none() {
        app.manage(Browser::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_phrase_is_searched_rather_than_navigated_to() {
        let url = resolve_input("how do i center a div").expect("search");
        assert!(url.as_str().contains("duckduckgo.com"), "{url}");
        assert!(url.as_str().contains("how+do+i+center+a+div"), "{url}");
    }

    #[test]
    fn a_real_address_is_navigated_to() {
        let url = resolve_input("example.com/docs").expect("url");
        assert_eq!(url.scheme(), "http");
        assert_eq!(url.host_str(), Some("example.com"));
        assert_eq!(url.path(), "/docs");
    }

    /// A scheme the user typed is honoured rather than searched for, which is what
    /// makes `https://localhost:8443` work and keeps it from becoming a query.
    #[test]
    fn an_explicit_scheme_is_kept_as_written() {
        let url = resolve_input("https://example.com").expect("url");
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host_str(), Some("example.com"));
    }

    /// No dot, so the host test cannot be the thing that recognises it. Without
    /// this case `localhost:5173` would be searched for as a phrase.
    #[test]
    fn localhost_with_a_port_is_an_address() {
        let url = resolve_input("localhost:5173").expect("url");
        assert_eq!(url.host_str(), Some("localhost"));
        assert_eq!(url.port(), Some(5173));
        assert!(!url.as_str().contains("duckduckgo"), "{url}");
    }

    #[test]
    fn localhost_without_a_port_is_an_address() {
        let url = resolve_input("localhost").expect("url");
        assert_eq!(url.host_str(), Some("localhost"));
    }

    #[test]
    fn a_loopback_address_is_an_address() {
        let url = resolve_input("127.0.0.1:8080").expect("url");
        assert_eq!(url.host_str(), Some("127.0.0.1"));
        assert_eq!(url.port(), Some(8080));
    }

    /// A dot is not enough. A sentence with a full stop in it is a question, and
    /// searching for it beats an error about a host that does not resolve.
    #[test]
    fn a_sentence_is_not_mistaken_for_a_host() {
        let url = resolve_input("what is this.error").expect("search");
        assert!(url.as_str().contains("duckduckgo"), "{url}");
    }

    /// `file://` outside the workspace is refused, because without the boundary a
    /// panel that could be pointed at one would be a way out of the workspace
    /// every other path in this app enforces. (A workspace test would need the
    /// process-global root; the refusal path needs none, so this covers it.)
    #[test]
    fn a_local_file_scheme_outside_the_workspace_is_refused() {
        let error = resolve_input("file:///C:/Users/secret.txt").expect_err("refused");
        assert!(error.contains("workspace"), "{error}");
    }

    #[test]
    fn an_unknown_scheme_is_refused() {
        for input in [
            "javascript:alert(1)",
            "data:text/html,<h1>x",
            "ftp://example.com",
        ] {
            let error = resolve_input(input).expect_err("refused");
            assert!(error.contains("Only http"), "{input}: {error}");
        }
    }

    #[test]
    fn an_empty_address_is_an_error_rather_than_a_blank_page() {
        assert!(resolve_input("   ").is_err());
    }

    fn history(urls: &[&str]) -> History {
        let mut history = History::default();
        for url in urls {
            history.push(url);
        }
        history
    }

    #[test]
    fn back_walks_what_was_visited() {
        let mut history = history(&["a", "b", "c"]);
        assert!(history.can_go_back());
        assert!(!history.can_go_forward());
        assert_eq!(history.step_back(), Some("b".to_string()));
        // Having stepped back, there is now something ahead to step to.
        assert_eq!(history.step_forward(), Some("c".to_string()));
        // And from the newest entry there is nothing ahead at all, which is the
        // case that makes a forward button look broken rather than inactive.
        assert_eq!(history.step_forward(), None);
    }

    #[test]
    fn back_and_forward_are_symmetric() {
        let mut history = history(&["a", "b"]);
        history.index = Some(0);
        assert!(!history.can_go_back());
        assert!(history.can_go_forward());
        assert_eq!(history.step_forward(), Some("b".to_string()));
    }

    /// The first page has nothing behind it, which is the case that makes a back
    /// button look broken on a freshly opened browser.
    #[test]
    fn the_first_page_has_nothing_behind_it() {
        let mut history = history(&["a"]);
        assert!(!history.can_go_back());
        assert_eq!(history.step_back(), None);
        assert_eq!(history.current(), "a");
    }

    #[test]
    fn an_empty_history_has_no_current_page() {
        let history = History::default();
        assert_eq!(history.current(), "");
        assert!(!history.can_go_back());
        assert!(!history.can_go_forward());
    }

    /// A redirect resolves to a page already in the list. Recording it as a new
    /// entry would make the user press back twice to leave one page.
    #[test]
    fn revisiting_a_page_does_not_add_a_second_entry() {
        let mut history = history(&["a", "b"]);
        history.push("a");
        assert_eq!(history.entries, vec!["a", "b"]);
        assert_eq!(history.current(), "a");
    }

    /// The bug this shape of `push` exists to prevent. `step` moves the index and
    /// then navigates, so the load that follows arrives here with a page already
    /// in the list -- and if that truncated the list, pressing back would delete
    /// the forward history and `forward` would never work again.
    #[test]
    fn stepping_back_keeps_the_forward_history() {
        let mut history = history(&["a", "b", "c"]);
        // What `step_back` does, followed by the `on_page_load` push for the page
        // it navigated to.
        assert_eq!(history.step_back(), Some("b".to_string()));
        history.push("b");

        assert_eq!(
            history.entries,
            vec!["a", "b", "c"],
            "back lost the history"
        );
        assert!(history.can_go_forward());
        assert_eq!(history.step_forward(), Some("c".to_string()));
    }

    /// `a, b, a` is a real thing a user does: go back, then pick the same page
    /// again. It must land on the *existing* `a` rather than a third entry, so
    /// the list stays two pages and `back` needs one press to leave.
    #[test]
    fn going_back_and_visiting_again_does_not_grow_the_list() {
        let mut history = history(&["a", "b"]);
        assert_eq!(history.step_back(), Some("a".to_string()));
        history.push("a");
        assert_eq!(history.entries, vec!["a", "b"]);
        assert_eq!(history.current(), "a");
        // `b` is still ahead, because the user went back to `a` rather than
        // navigating away from it. This is the same rule that makes the back and
        // forward buttons work at all.
        assert!(history.can_go_forward());
        assert_eq!(history.step_forward(), Some("b".to_string()));
    }

    /// Navigating after going back abandons the forward stack. Keeping it would
    /// let "forward" replay a page the user deliberately left.
    #[test]
    fn a_new_navigation_drops_the_forward_pages() {
        let mut history = history(&["a", "b", "c"]);
        history.index = Some(0);
        assert!(history.can_go_forward());
        history.push("d");
        assert!(!history.can_go_forward());
        assert_eq!(history.entries, vec!["a", "d"]);
        assert_eq!(history.current(), "d");
    }

    /// `back` and `forward` move the index themselves, because the navigation that
    /// follows fires `on_page_load` and pushes the page -- without the step the
    /// index would land back on the entry it just left.
    #[test]
    fn stepping_moves_the_index_rather_than_only_reporting() {
        let mut history = history(&["a", "b", "c"]);
        assert_eq!(history.step_back(), Some("b".to_string()));
        assert_eq!(history.current(), "b");
        assert_eq!(history.step_forward(), Some("c".to_string()));
        assert_eq!(history.current(), "c");
    }

    #[test]
    fn stepping_past_the_end_is_nothing_rather_than_a_panic() {
        let mut history = history(&["a"]);
        assert_eq!(history.step_back(), None);
        // Still on the only page, having not moved.
        assert_eq!(history.current(), "a");
        assert_eq!(history.step_forward(), None);
        assert_eq!(history.current(), "a");
    }

    /// A search is a navigation like any other, so `back` undoes it and the
    /// browser is not stuck on a page with no history behind it.
    #[test]
    fn a_search_is_an_entry_like_any_other_page() {
        let url = resolve_input("tauri webview").expect("search");
        let mut history = History::default();
        history.push(url.as_str());
        assert!(!history.can_go_back());
        assert_eq!(history.current(), url.as_str());
    }
}
