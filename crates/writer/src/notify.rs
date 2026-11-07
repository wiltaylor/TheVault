//! Notifications for the user, from scripts, agents and the server (see `state`'s notifications).
//! The writer adds the one check `state.db` cannot make itself: a notification's note exists.

use state::{NewNotification, Notification, SourceLimit};

use crate::{Error, Result, Writer};

impl Writer {
    /// Store notification `n`. The note it names must be a live note: a missing or trashed one
    /// is a [`Error::BadRequest`], as are a bad title, body or url.
    pub fn notify(&self, n: &NewNotification) -> Result<Notification> {
        self.notify_within(n, &[])
    }

    /// Store notification `n` as [`Writer::notify`] does, unless its source is over one of
    /// `limits`: then [`Error::State`] holds [`state::Error::NotificationLimit`] and nothing is
    /// stored (see [`state::State::add_notification_within`]).
    pub fn notify_within(&self, n: &NewNotification, limits: &[SourceLimit]) -> Result<Notification> {
        if let Some(id) = n
            .note_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            && self.type_and_title(id)?.is_none()
        {
            return Err(Error::BadRequest(if self.state().in_trash(id)? {
                format!("note {id} is in the trash")
            } else {
                format!("there is no note {id}")
            }));
        }
        self.state().add_notification_within(n, limits).map_err(|e| match e {
            state::Error::InvalidNotification(m) => Error::BadRequest(m),
            other => Error::State(other),
        })
    }
}
