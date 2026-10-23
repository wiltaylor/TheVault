//! Notifications: short messages for the user from scripts, agents and the server itself, shown
//! under the web app's bell. Each can point at a note or at a page of the app. They are app
//! state, not vault content, so they live here rather than in the vault files.
//!
//! The list is bounded: adding one first removes read notifications older than
//! [`READ_RETENTION_MS`], then the oldest beyond [`MAX_NOTIFICATIONS`], read or not.

use rusqlite::{OptionalExtension, Transaction, params};

use crate::{Error, LiveEvent, Result, State};

/// The most notifications kept.
pub const MAX_NOTIFICATIONS: usize = 500;

/// How long a read notification is kept, in milliseconds: 30 days.
pub const READ_RETENTION_MS: i64 = 30 * 24 * 60 * 60 * 1000;

/// The longest title, in characters.
pub const MAX_TITLE: usize = 200;

/// The longest body, in characters.
pub const MAX_BODY: usize = 2000;

/// The longest in-app path, in characters.
pub const MAX_URL: usize = 1000;

/// The longest source label, in characters.
pub const MAX_SOURCE: usize = 200;

/// What a new notification says and where it came from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewNotification {
    pub title: String,
    pub body: Option<String>,
    /// The note it opens. The caller checks that the note exists.
    pub note_id: Option<String>,
    /// The page of the app it opens when it names no note, such as `/settings/scheduled/digest`.
    pub url: Option<String>,
    /// Who made it: `task:<name>`, `handler:<type>.<event>`, `api`, `token:<name>` or `system`.
    pub source: String,
}

/// One stored notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    /// A ULID, so ids sort by creation.
    pub id: String,
    pub created_ms: i64,
    pub title: String,
    pub body: Option<String>,
    pub note_id: Option<String>,
    pub url: Option<String>,
    pub source: String,
    /// When it was marked read, or `None` while unread.
    pub read_ms: Option<i64>,
}

/// One page of notifications, newest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationPage {
    pub items: Vec<Notification>,
    /// The cursor of the next page, or `None` on the last.
    pub next: Option<String>,
}

const COLUMNS: &str = "id, created_ms, title, body, note_id, url, source, read_ms";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Notification> {
    Ok(Notification {
        id: r.get(0)?,
        created_ms: r.get(1)?,
        title: r.get(2)?,
        body: r.get(3)?,
        note_id: r.get(4)?,
        url: r.get(5)?,
        source: r.get(6)?,
        read_ms: r.get(7)?,
    })
}

fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidNotification(message.into())
}

/// `s` trimmed, or `None` when that leaves nothing.
fn optional(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Whether `url` is a path inside the app: it starts with one `/`, is not an API or login path,
/// and holds no spaces or control characters.
pub fn is_app_path(url: &str) -> bool {
    url.starts_with('/')
        && !url.starts_with("//")
        && !url.starts_with("/api/")
        && url != "/api"
        && !url.starts_with("/login")
        && !url
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
}

/// The notification `n` describes, checked: a title of 1 to [`MAX_TITLE`] characters, a body of
/// at most [`MAX_BODY`], and a url that is an app path.
fn checked(n: &NewNotification) -> Result<NewNotification> {
    let title = n.title.trim();
    if title.is_empty() {
        return Err(invalid("a notification needs a title"));
    }
    if title.chars().count() > MAX_TITLE {
        return Err(invalid(format!(
            "the title is longer than {MAX_TITLE} characters"
        )));
    }
    let body = optional(n.body.as_deref());
    if body.as_ref().is_some_and(|b| b.chars().count() > MAX_BODY) {
        return Err(invalid(format!(
            "the body is longer than {MAX_BODY} characters"
        )));
    }
    let url = optional(n.url.as_deref());
    if let Some(u) = &url
        && (u.chars().count() > MAX_URL || !is_app_path(u))
    {
        return Err(invalid(format!(
            "{u:?} is not a path in the app, such as /settings/scheduled/digest"
        )));
    }
    let source = n.source.trim();
    if source.is_empty() || source.chars().count() > MAX_SOURCE {
        return Err(invalid(
            "a notification's source must be 1 to 200 characters",
        ));
    }
    Ok(NewNotification {
        title: title.to_owned(),
        body,
        note_id: optional(n.note_id.as_deref()),
        url,
        source: source.to_owned(),
    })
}

/// The ULID of a notification made at `now_ms`, after every id `tx` already holds, so ids keep
/// creation order within a millisecond and when the clock moves back.
fn next_id(tx: &Transaction<'_>, now_ms: i64) -> Result<String> {
    let mut random = [0u8; 10];
    crate::random_bytes(&mut random);
    let random = random
        .iter()
        .fold(0u128, |acc, b| (acc << 8) | u128::from(*b));
    let ms = u64::try_from(now_ms).unwrap_or(0);
    let id = ulid::Ulid::from_parts(ms, random);
    let last: Option<String> = tx
        .query_row(
            "SELECT id FROM notifications ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let last = last.and_then(|l| ulid::Ulid::from_string(&l).ok());
    Ok(match last {
        Some(last) if last >= id => last
            .increment()
            .ok_or_else(|| Error::Corrupt("the notification ids have run out".into()))?
            .to_string(),
        _ => id.to_string(),
    })
}

impl State {
    /// Store a notification and return it, removing old ones first (see the module docs).
    pub fn add_notification(&self, n: &NewNotification) -> Result<Notification> {
        let n = checked(n)?;
        let now = self.now();
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM notifications WHERE read_ms IS NOT NULL AND read_ms < ?1",
            [now.saturating_sub(READ_RETENTION_MS)],
        )?;
        // Room for the new one within the cap: the oldest go first.
        tx.execute(
            "DELETE FROM notifications WHERE id NOT IN
                (SELECT id FROM notifications ORDER BY id DESC LIMIT ?1)",
            [MAX_NOTIFICATIONS as i64 - 1],
        )?;
        let id = next_id(&tx, now)?;
        tx.execute(
            "INSERT INTO notifications (id, created_ms, title, body, note_id, url, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, now, n.title, n.body, n.note_id, n.url, n.source],
        )?;
        tx.commit()?;
        self.events.publish(LiveEvent::NotificationCreated {
            id: id.clone(),
            unread: unread(&conn)?,
        });
        drop(conn);
        Ok(Notification {
            id,
            created_ms: now,
            title: n.title,
            body: n.body,
            note_id: n.note_id,
            url: n.url,
            source: n.source,
            read_ms: None,
        })
    }

    /// Change an unread notification's title and body, checked as [`State::add_notification`]
    /// checks them. Returns `None`, changing nothing, when the notification is gone or read.
    pub fn update_notification(
        &self,
        id: &str,
        title: &str,
        body: Option<&str>,
    ) -> Result<Option<Notification>> {
        let Some(old) = self.notification(id)? else {
            return Ok(None);
        };
        if old.read_ms.is_some() {
            return Ok(None);
        }
        let n = checked(&NewNotification {
            title: title.to_owned(),
            body: body.map(str::to_owned),
            note_id: old.note_id.clone(),
            url: old.url.clone(),
            source: old.source.clone(),
        })?;
        let conn = self.conn();
        let changed = conn.execute(
            "UPDATE notifications SET title = ?2, body = ?3 WHERE id = ?1 AND read_ms IS NULL",
            params![id, n.title, n.body],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        self.events.publish(LiveEvent::NotificationUpdated {
            id: id.to_owned(),
            unread: unread(&conn)?,
        });
        drop(conn);
        Ok(Some(Notification {
            title: n.title,
            body: n.body,
            ..old
        }))
    }

    /// One notification by id.
    pub fn notification(&self, id: &str) -> Result<Option<Notification>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {COLUMNS} FROM notifications WHERE id = ?1"),
                [id],
                row,
            )
            .optional()?)
    }

    /// A page of at most `limit` notifications, newest first, only unread ones when `unread`.
    /// `cursor` is the `next` of the page before, or `None` for the first.
    pub fn notifications(
        &self,
        unread: bool,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<NotificationPage> {
        let limit = limit.clamp(1, MAX_NOTIFICATIONS);
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM notifications
             WHERE (?1 = 0 OR read_ms IS NULL) AND (?2 IS NULL OR id < ?2)
             ORDER BY id DESC LIMIT ?3"
        ))?;
        let mut items = stmt
            .query_map(params![unread, cursor, limit as i64 + 1], row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let next = if items.len() > limit {
            items.truncate(limit);
            items.last().map(|n| n.id.clone())
        } else {
            None
        };
        Ok(NotificationPage { items, next })
    }

    /// How many notifications are unread.
    pub fn unread_notifications(&self) -> Result<u64> {
        unread(&self.conn())
    }

    /// Mark one notification read. Marking a read one again keeps when it was first read, and
    /// tells no one.
    pub fn mark_notification_read(&self, id: &str) -> Result<Notification> {
        let now = self.now();
        let conn = self.conn();
        let marked = conn.execute(
            "UPDATE notifications SET read_ms = ?2 WHERE id = ?1 AND read_ms IS NULL",
            params![id, now],
        )?;
        if marked > 0 {
            self.events.publish(LiveEvent::NotificationRead {
                id: Some(id.to_owned()),
                unread: unread(&conn)?,
            });
        }
        drop(conn);
        self.notification(id)?
            .ok_or_else(|| Error::UnknownNotification(id.to_owned()))
    }

    /// Mark every unread notification read. Returns how many were unread.
    pub fn mark_all_notifications_read(&self) -> Result<usize> {
        let now = self.now();
        let conn = self.conn();
        let marked = conn.execute(
            "UPDATE notifications SET read_ms = ?1 WHERE read_ms IS NULL",
            [now],
        )?;
        if marked > 0 {
            self.events.publish(LiveEvent::NotificationRead {
                id: None,
                unread: 0,
            });
        }
        Ok(marked)
    }

    /// Remove one notification, read or not.
    pub fn delete_notification(&self, id: &str) -> Result<()> {
        let conn = self.conn();
        match conn.execute("DELETE FROM notifications WHERE id = ?1", [id])? {
            0 => Err(Error::UnknownNotification(id.to_owned())),
            _ => {
                self.events.publish(LiveEvent::NotificationDismissed {
                    id: id.to_owned(),
                    unread: unread(&conn)?,
                });
                Ok(())
            }
        }
    }

    /// Remove every read notification. Returns how many went.
    pub fn clear_read_notifications(&self) -> Result<usize> {
        let conn = self.conn();
        let cleared = conn.execute("DELETE FROM notifications WHERE read_ms IS NOT NULL", [])?;
        if cleared > 0 {
            self.events.publish(LiveEvent::NotificationCleared {
                cleared,
                unread: unread(&conn)?,
            });
        }
        Ok(cleared)
    }
}

/// How many notifications are unread.
fn unread(conn: &rusqlite::Connection) -> Result<u64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM notifications WHERE read_ms IS NULL",
        [],
        |r| r.get(0),
    )?)
}
