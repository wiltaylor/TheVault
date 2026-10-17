//! The fake Google Calendar API, under [`super::CALENDAR_PATH`]. Empty for now: the calendar
//! sync and write-back tickets add calendarList, events list with sync tokens (and 410),
//! instances, insert, patch with `If-Match` (412 on a stale etag) and delete here, with their
//! builder switches in an `impl FakeGoogleBuilder` block and their test controls in an
//! `impl FakeGoogle` block, and answer them from [`handle`] after checking
//! [`super::authorized`].

use super::Shared;
use crate::standin::{Reply, Request};

/// How the fake Calendar starts.
#[derive(Debug, Clone, Default)]
pub(super) struct Config {}

/// The fake Calendar's calendars and events.
#[derive(Debug, Default)]
pub(super) struct State {}

impl State {
    pub(super) fn new(_config: Config) -> State {
        State {}
    }
}

/// Answer a Calendar request, or `None` when `request` is not one.
pub(super) fn handle(_shared: &Shared, _request: &Request) -> Option<Reply> {
    None
}
