//! Notifications for the user, from scripts, agents and the server (see `state`'s notifications).
//! The writer adds the one check `state.db` cannot make itself: a notification's note exists.

use state::{NewNotification, Notification, SourceLimit, SourceMatch};

use crate::{Error, Result, Writer};

/// The source prefix of every agent's notifications (`agent:<name>`).
pub const AGENT_SOURCES: &str = "agent:";

/// The most notifications one agent sends in an hour.
pub const MAX_PER_HOUR: usize = 10;

/// The most notifications one agent sends in a day.
pub const MAX_PER_DAY: usize = 30;

/// The most notifications all agents together send in a day, on top of [`MAX_PER_DAY`] for each.
pub const MAX_PER_DAY_ALL_AGENTS: usize = 60;

const HOUR_MS: i64 = 60 * 60 * 1000;
const DAY_MS: i64 = 24 * HOUR_MS;

/// The limits on a notification from an agent, whether the agent's own `notify_user` call or a
/// script an agent started (TV-76): the hour and day counts of its source, then the day count of
/// all agents together (TV-75). Checked in the one transaction that stores the notification.
pub const AGENT_LIMITS: [SourceLimit; 3] = [
    SourceLimit {
        window_ms: HOUR_MS,
        max: MAX_PER_HOUR,
        sources: SourceMatch::Own,
    },
    SourceLimit {
        window_ms: DAY_MS,
        max: MAX_PER_DAY,
        sources: SourceMatch::Own,
    },
    SourceLimit {
        window_ms: DAY_MS,
        max: MAX_PER_DAY_ALL_AGENTS,
        sources: SourceMatch::Prefix(AGENT_SOURCES),
    },
];

/// Why [`AGENT_LIMITS`] refused a notification: `by` is the agent's source, or [`AGENT_SOURCES`]
/// for the cap of all agents together. Names the limit and the count, not the note.
pub fn agent_limit_message(by: &str, window_ms: i64, count: usize, max: usize) -> String {
    if by == AGENT_SOURCES {
        format!(
            "all agents together sent {count} notifications in the last 24 hours, and the limit \
             is {max} a day"
        )
    } else if window_ms == HOUR_MS {
        format!(
            "this agent sent {count} notifications in the last hour, and the limit is {max} an hour"
        )
    } else {
        format!(
            "this agent sent {count} notifications in the last 24 hours, and the limit is {max} a day"
        )
    }
}

impl Writer {
    /// Store notification `n`. The note it names must be a live note: a missing or trashed one
    /// is a [`Error::BadRequest`], as are a bad title, body or url.
    pub fn notify(&self, n: &NewNotification) -> Result<Notification> {
        self.notify_within(n, &[])
    }

    /// Store notification `n` as [`Writer::notify`] does, unless its source is over one of
    /// `limits`: then [`Error::State`] holds [`state::Error::NotificationLimit`] and nothing is
    /// stored (see [`state::State::add_notification_within`]).
    pub fn notify_within(
        &self,
        n: &NewNotification,
        limits: &[SourceLimit],
    ) -> Result<Notification> {
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
        self.state()
            .add_notification_within(n, limits)
            .map_err(|e| match e {
                state::Error::InvalidNotification(m) => Error::BadRequest(m),
                other => Error::State(other),
            })
    }
}
