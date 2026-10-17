//! The fake Gmail API, under [`super::GMAIL_PATH`]. Empty for now: the Gmail sync ticket adds
//! labels, messages list, message get, attachments and history (with a switch to answer 404)
//! here, with their builder switches in an `impl FakeGoogleBuilder` block and their test
//! controls in an `impl FakeGoogle` block, and answers them from [`handle`] after checking
//! [`super::authorized`].

use super::Shared;
use crate::standin::{Reply, Request};

/// How the fake Gmail starts.
#[derive(Debug, Clone, Default)]
pub(super) struct Config {}

/// The fake Gmail's mailbox.
#[derive(Debug, Default)]
pub(super) struct State {}

impl State {
    pub(super) fn new(_config: Config) -> State {
        State {}
    }
}

/// Answer a Gmail request, or `None` when `request` is not one.
pub(super) fn handle(_shared: &Shared, _request: &Request) -> Option<Reply> {
    None
}
