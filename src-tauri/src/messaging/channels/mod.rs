//! The transports, and the one line it takes to add another.
//!
//! A channel is constructed with the database (it may need to read its own stored
//! configuration) and nothing else. The hub injects itself when the loop starts, so
//! a channel never holds a strong reference to the thing that owns it.

pub mod telegram;

use std::sync::Arc;

use crate::database::Database;

use super::adapter::Channel;

/// Every transport this build knows.
///
/// Discord, Slack or a webhook receiver would be one more `Arc::new(...)` here and
/// nothing else: the core never names a channel, it only asks the list.
pub fn registry(database: &Arc<Database>) -> Vec<Arc<dyn Channel>> {
    vec![Arc::new(telegram::Telegram::new(Arc::clone(database)))]
}
