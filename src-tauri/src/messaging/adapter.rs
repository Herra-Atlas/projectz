//! What a messaging channel has to be able to do.
//!
//! # A trait, not a match
//!
//! The core is deliberately blind to which transport it is talking to. Adding a
//! second one is a new folder under `channels/` and one line in the registry --
//! not a new match arm in the run path, the command list and the status page.
//!
//! # Why the methods return boxed futures
//!
//! `async fn` in a trait is not object-safe on the Rust this project builds with,
//! and an `async_trait` dependency for five methods is not worth a crate. A boxed
//! future is the smallest thing that lets `Arc<dyn Channel>` exist, and the boxing
//! cost is nothing beside a network round trip.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::messaging::types::{
    Button, Capabilities, ChannelStatus, DownloadedFile, InboundFile, OutboundFile, SentMessage,
};
use crate::messaging::Messaging;

/// A future a trait object can return.
pub type ChannelFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// One transport.
pub trait Channel: Send + Sync {
    /// The stable id stored in the settings record and used in event keys.
    fn id(&self) -> &'static str;

    /// The name the Settings page draws.
    fn name(&self) -> &'static str;

    /// What this transport can draw.
    fn capabilities(&self) -> Capabilities;

    /// Whether the master switch has this channel on. Set by the hub.
    fn set_enabled(&self, enabled: bool);

    /// Polls until the process ends.
    ///
    /// The implementation is expected to park itself while disabled rather than
    /// return: the loop outlives every settings change, which is what makes a
    /// token edit take effect without restarting the app.
    fn listen(self: Arc<Self>, hub: Arc<Messaging>) -> ChannelFuture<'static, ()>;

    /// Sends one message, optionally with buttons beneath it.
    fn send<'a>(
        &'a self,
        to: &'a str,
        text: &'a str,
        buttons: &'a [Button],
    ) -> ChannelFuture<'a, Result<SentMessage, String>>;

    /// Fetches the bytes of a file named by an inbound message.
    fn download<'a>(
        &'a self,
        file: &'a InboundFile,
    ) -> ChannelFuture<'a, Result<DownloadedFile, String>>;

    /// Sends one workspace file back to the chat.
    fn send_file<'a>(
        &'a self,
        to: &'a str,
        file: &'a OutboundFile,
    ) -> ChannelFuture<'a, Result<SentMessage, String>>;

    /// Removes one message the app sent, where the transport supports it.
    ///
    /// Used for the transient working message once an answer has landed. A channel
    /// that cannot delete may return `Ok(())`; the core has already sent the answer.
    fn delete<'a>(
        &'a self,
        to: &'a str,
        message_id: &'a str,
    ) -> ChannelFuture<'a, Result<(), String>>;

    /// Rewrites a message already sent. A channel without the capability may
    /// return `Ok(())` and do nothing -- the core treats editing as an
    /// optimisation, never as the only way to say something.
    fn edit<'a>(
        &'a self,
        to: &'a str,
        message_id: &'a str,
        text: &'a str,
        buttons: &'a [Button],
    ) -> ChannelFuture<'a, Result<(), String>>;

    /// Shows "typing" for a moment, where the transport has such a thing.
    fn typing<'a>(&'a self, to: &'a str) -> ChannelFuture<'a, Result<(), String>>;

    /// Acknowledges a button press, which most transports require or the press
    /// appears to hang.
    fn answer<'a>(
        &'a self,
        action_id: &'a str,
        text: Option<&'a str>,
    ) -> ChannelFuture<'a, Result<(), String>>;

    /// What this channel is doing, for the Settings page.
    fn status(&self) -> ChannelStatus;

    /// The channel's own configuration, already redacted for the page.
    ///
    /// A secret is never in here. The page is told whether one is saved, not what
    /// it is -- the same rule the provider list follows for an API key.
    fn config(&self) -> serde_json::Value;

    /// Validates and stores the channel's configuration.
    ///
    /// Validation is the adapter's, because only it knows what a usable token is.
    /// An error leaves the stored configuration alone, so a typo cannot replace a
    /// token that was working.
    fn configure<'a>(
        &'a self,
        config: serde_json::Value,
    ) -> ChannelFuture<'a, Result<serde_json::Value, String>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default a channel without an explicit opinion gets: plain messages,
    /// split at a conservative length. Declared here as a reminder that a
    /// capability is a fact about the transport, and a transport that cannot do
    /// something simply says so.
    #[test]
    fn capabilities_are_plain_data() {
        let capabilities = Capabilities {
            buttons: false,
            edit: false,
            typing: false,
            max_chars: 2_000,
            files: false,
            delete: false,
        };
        assert_eq!(capabilities.max_chars, 2_000);
        assert!(!capabilities.buttons);
    }

    /// Button data is the adapter's to carry unchanged; the core only agrees on
    /// the prefixes. This pins the boundary rather than the spelling.
    #[test]
    fn a_button_is_a_label_and_opaque_data() {
        let button = Button::new("Allow", "ap:1234:y");
        assert_eq!(button.label, "Allow");
        assert!(button.data.starts_with("ap:"));
    }
}
