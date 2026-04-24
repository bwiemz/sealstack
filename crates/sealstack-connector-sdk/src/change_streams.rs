//! Helpers for building the stream types the [`super::Connector`] trait returns.

use super::{ChangeEvent, ChangeStream, Resource, ResourceStream};
use futures::stream;

/// Build a [`ResourceStream`] from an owned `Vec`.
///
/// Fine for connectors whose source fits in memory. Large sources should
/// implement a lazy stream directly.
#[must_use]
pub fn resource_stream(resources: Vec<Resource>) -> ResourceStream {
    Box::pin(stream::iter(resources))
}

/// Build a [`ChangeStream`] from an owned `Vec`.
#[must_use]
pub fn change_stream(events: Vec<ChangeEvent>) -> ChangeStream {
    Box::pin(stream::iter(events))
}
