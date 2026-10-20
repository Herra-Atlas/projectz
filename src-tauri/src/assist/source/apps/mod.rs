//! Windows applications, when they are picked up.
//!
//! # Why this file is documentation and not code
//!
//! Nothing here is built, and nothing here pretends to be: a source that is not implemented
//! is not listed in settings either. What this file holds is the reasoning, written down
//! where the code that needs it will be -- so picking the work up is a change to this file
//! plus an overlay window, with no reshaping of the pipeline.
//!
//! # What it takes, in order of how much it costs
//!
//! **UI Automation.** Every window that publishes an accessibility tree can be asked for its
//! text and the bounds of each text range, which is exact, cheap (a few milliseconds), and
//! needs no screenshot, no OCR and no vision model. It is also the only path that gives a
//! *position* good enough to anchor a card beside the text it is about, and the only one that
//! reports whether a field is a password field -- which is the difference between reading a
//! puzzle and reading someone's login. It is unavailable exactly where it is most wanted:
//! games and anything drawing to a canvas publish nothing at all.
//!
//! **A screenshot, when there is no tree.** Two mechanisms, and the choice is not optional:
//! `BitBlt` against the window is the simple one and returns black for anything on a modern
//! graphics surface, so a game or a hardware-accelerated window needs DXGI desktop duplication
//! instead, which is a different API with a different failure mode (it fails while a
//! fullscreen exclusive application holds the display).
//!
//! **The model, for a picture.** Text recovered as pixels has to be read by something. Doing
//! that on-device is the cheap route once a small vision model is loaded, and it is the route
//! that keeps a screenshot of the user's screen on the user's machine.
//!
//! # What is not a solution
//!
//! A transparent always-on-top window can be drawn over an ordinary bordered window, and
//! cannot be drawn over an exclusive-fullscreen presentation at all -- the compositor is not
//! showing anything else. That is a property of the operating system rather than of this app,
//! so it belongs in what the feature promises: applications are helped *windowed*, and
//! fullscreen games are not helped. Saying so is better than a card that silently fails to
//! appear.
