//! The browser's tabs.
//!
//! Chrome calls them *targets*, and a target listing is the only complete answer
//! to "what is open": the user can open a tab in the window they are watching, a
//! page can open one for itself, and the model can ask for one. None of those go
//! through this app, so the list is read back from the browser whenever a call
//! starts rather than kept alongside it -- a list kept here would be wrong the
//! first time the user clicked a link.
//!
//! # Re-syncing attaches and detaches, it does not rebuild
//!
//! A tab that is still open keeps the attachment it already had. Re-attaching
//! would enable every domain again and lose this tab's queued events, and the
//! tab's numbers would be assigned afresh for a page that had not changed. So a
//! sync carries the existing [`Page`] objects across by target id, attaches only
//! what is genuinely new, and forgets what is gone.
//!
//! # Order is the browser's, not ours
//!
//! Tab numbers are positions in the browser's own listing. That is what makes
//! `tab` in a tool call mean something stable to a reader: tab 0 is the first tab
//! in the window the user is looking at, and it stays tab 0 until something is
//! closed or opened ahead of it.

use std::sync::Arc;

use serde_json::{json, Value};

use super::cdp::Connection;
use super::page::{self, Page};

/// Every open tab, and which one is being driven.
pub struct Tabs {
    pages: Vec<Page>,
    active: usize,
}

impl Tabs {
    /// Attaches to the tab the browser opened with.
    pub async fn attach_first(conn: &Arc<Connection>) -> Result<Self, String> {
        let listing = conn.call("Target.getTargets", json!({}), None).await?;
        let targets = page_targets(&listing);
        let (target_id, url) = targets
            .first()
            .cloned()
            .ok_or("the browser started with no page open")?;
        let page = page::attach(conn, &target_id, &url).await?;
        Ok(Self {
            pages: vec![page],
            active: 0,
        })
    }

    /// Brings the tab list up to date with the browser.
    ///
    /// Cheap in the common case: one listing, no attachments, and the tab that
    /// was being driven is still the one being driven.
    pub async fn sync(&mut self, conn: &Arc<Connection>) -> Result<(), String> {
        let listing = conn.call("Target.getTargets", json!({}), None).await?;
        let wanted = page_targets(&listing);

        let mut carried: Vec<Option<Page>> = std::mem::take(&mut self.pages)
            .into_iter()
            .map(Some)
            .collect();
        let active_id = carried
            .get(self.active)
            .and_then(|slot| slot.as_ref())
            .map(|page| page.target_id.clone());

        let mut pages = Vec::new();
        for (target_id, url) in &wanted {
            let existing = carried.iter_mut().find(|slot| {
                slot.as_ref()
                    .is_some_and(|page| &page.target_id == target_id)
            });
            match existing.and_then(|slot| slot.take()) {
                Some(mut page) => {
                    // The address is refreshed here rather than asked for later:
                    // it came with the listing, and a tab the model is not on is
                    // only ever named.
                    page.url = url.clone();
                    pages.push(page);
                }
                None => match page::attach(conn, target_id, url).await {
                    Ok(page) => pages.push(page),
                    // One tab that will not attach is not the end of the others.
                    // A devtools window, a page in the middle of being torn down,
                    // or a renderer that has crashed would otherwise cost the run
                    // every tab that does work.
                    Err(error) => {
                        tracing::debug!(target = %target_id, error = %error, "a tab could not be attached")
                    }
                },
            }
        }

        // Whatever is left was closed. Its queue is dropped with it, so a later
        // session id cannot receive the events of a tab that no longer exists.
        for closed in carried.into_iter().flatten() {
            conn.unroute(closed.session()).await;
        }

        // Nobody is on a tab that closed: the position is taken by whatever the
        // browser now lists there, or by the first tab if the list is shorter.
        self.active = active_id
            .and_then(|id| pages.iter().position(|page| page.target_id == id))
            .unwrap_or(0);
        self.pages = pages;
        Ok(())
    }

    /// The tab being driven.
    pub fn active(&mut self) -> Option<&mut Page> {
        self.pages.get_mut(self.active)
    }

    /// Switches to a tab by its number, reporting whether there was one.
    pub fn select(&mut self, index: usize) -> bool {
        if index < self.pages.len() {
            self.active = index;
            true
        } else {
            false
        }
    }

    /// Switches to a tab by the browser's id for it.
    pub fn follow(&mut self, target_id: &str) -> Option<usize> {
        let index = self
            .pages
            .iter()
            .position(|page| page.target_id == target_id)?;
        self.active = index;
        Some(index)
    }

    /// Whether there is no tab at all, which is a browser that has just exited.
    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }

    /// Which tab is being driven.
    pub fn active_index(&self) -> usize {
        self.active
    }

    /// Every tab's address, in the browser's order.
    pub fn urls(&self) -> Vec<String> {
        self.pages.iter().map(|page| page.url.clone()).collect()
    }
}

/// Every real tab in a target listing, as `(targetId, url)`.
///
/// Filtered on purpose. A listing also holds service workers, out-of-process
/// iframes, and Chrome's own devtools front-end; attaching to one of those would
/// hand the driver a page that cannot be navigated or read, and it would then be
/// counted as a tab the model could switch to.
fn page_targets(listing: &Value) -> Vec<(String, String)> {
    listing
        .get("targetInfos")
        .and_then(Value::as_array)
        .map(|infos| {
            infos
                .iter()
                .filter(|info| info.get("type").and_then(Value::as_str) == Some("page"))
                .filter(|info| {
                    !info
                        .get("url")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .starts_with("devtools://")
                })
                .map(|info| {
                    (
                        info.get("targetId")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        info.get("url")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape Chrome really sends, trimmed to the fields that matter.
    fn listing() -> Value {
        json!({
            "targetInfos": [
                { "targetId": "A", "type": "page", "url": "https://example.com/" },
                { "targetId": "B", "type": "service_worker", "url": "https://example.com/sw.js" },
                { "targetId": "C", "type": "page", "url": "devtools://devtools/bundled/inspector.html" },
                { "targetId": "D", "type": "page", "url": "about:blank" },
                { "targetId": "E", "type": "iframe", "url": "https://ads.example/frame" }
            ]
        })
    }

    /// Only pages, and not the devtools front-end: a worker or an iframe attached
    /// as a tab would be a tab that cannot be navigated or read, and the model
    /// would be offered it as a place to go.
    #[test]
    fn only_real_pages_are_tabs() {
        let tabs = page_targets(&listing());
        assert_eq!(
            tabs,
            vec![
                ("A".to_string(), "https://example.com/".to_string()),
                ("D".to_string(), "about:blank".to_string()),
            ]
        );
    }

    #[test]
    fn a_listing_with_nothing_in_it_is_no_tabs_rather_than_a_failure() {
        assert!(page_targets(&json!({})).is_empty());
        assert!(page_targets(&json!({ "targetInfos": [] })).is_empty());
    }

    #[test]
    fn a_tab_is_switched_to_by_its_number_and_only_when_there_is_one() {
        let mut tabs = Tabs {
            pages: Vec::new(),
            active: 0,
        };
        assert!(!tabs.select(0), "there is nothing to switch to");
        assert!(tabs.is_empty());
        assert_eq!(tabs.active_index(), 0);
    }
}
