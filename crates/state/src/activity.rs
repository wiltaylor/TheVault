//! The activity log (activity Y1, Y2, Y6): an append-only record of what happened in the vault,
//! by whom, for the Log page. Every vault change and all AI activity is recorded here, one indexed
//! insert per event.
//!
//! An entry's id is a ULID, which is also its order. Its summary is one line of at most
//! [`MAX_SUMMARY`] characters and its detail a small JSON object of at most [`MAX_DETAIL`] bytes;
//! callers never put secret values or message text in either.
//!
//! [`State::record_activity`] never fails the operation that made the entry: an error is logged
//! and dropped. A recorded entry is published as [`LiveEvent::Activity`].
//!
//! Retention: entries older than [`RETENTION_MS`], and beyond the newest [`MAX_ENTRIES`], are
//! pruned by [`State::prune_activity`], which the server calls as it starts, and once a day after
//! that, on the first record a day after `state.db` was opened or last pruned.

use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::{Map, Value};

use crate::{Author, Error, LiveEvent, Result, State};

/// How long an entry is kept, in milliseconds: 90 days.
pub const RETENTION_MS: i64 = 90 * 24 * 60 * 60 * 1000;

/// The most entries kept.
pub const MAX_ENTRIES: usize = 200_000;

/// The longest summary, in characters.
pub const MAX_SUMMARY: usize = 300;

/// The largest detail, in bytes of JSON.
pub const MAX_DETAIL: usize = 4096;

/// The largest page [`State::activity`] answers.
pub const MAX_PAGE: usize = 200;

/// The page size when none is asked for.
pub const DEFAULT_PAGE: usize = 100;

/// How long a burst of one actor's edits of one note keeps coalescing into one `note.update`
/// entry, from the entry's first edit.
pub const COALESCE_MS: i64 = 60 * 1000;

/// How often the log is pruned while the server runs.
const PRUNE_EVERY_MS: i64 = 24 * 60 * 60 * 1000;

/// The categories an entry belongs to.
pub const CATEGORIES: [&str; 7] = [
    "ai",
    "notes",
    "definitions",
    "automation",
    "outbox",
    "sync",
    "auth",
];

/// A new entry for [`State::record_activity`].
#[derive(Debug, Clone, PartialEq)]
pub struct NewActivity {
    /// Dotted, such as `note.update` or `ai.tool`.
    pub kind: String,
    /// One of [`CATEGORIES`].
    pub category: &'static str,
    /// `session`, `token`, `handler`, `task`, `agent`, `device` or `server`. A token that belongs
    /// to a paired device is recorded as the device, by its name.
    pub actor_kind: String,
    pub actor: String,
    pub summary: String,
    /// What the entry links to, such as `("note", "<id>")` or `("thread", "<id>")`.
    pub subject: Option<(String, String)>,
    /// A small JSON object.
    pub detail: Value,
    /// Fold this entry into the newest entry of the same subject when that one has the same kind
    /// and actor and began less than [`COALESCE_MS`] ago, counting it in `detail.edits`.
    pub coalesce: bool,
}

impl NewActivity {
    /// An entry by `author`.
    pub fn new(
        kind: impl Into<String>,
        category: &'static str,
        author: &Author,
        summary: impl Into<String>,
    ) -> NewActivity {
        let (actor_kind, actor) = author.parts();
        NewActivity::by(kind, category, actor_kind, actor, summary)
    }

    /// An entry by the actor of `actor_kind` named `actor`.
    pub fn by(
        kind: impl Into<String>,
        category: &'static str,
        actor_kind: &str,
        actor: &str,
        summary: impl Into<String>,
    ) -> NewActivity {
        NewActivity {
            kind: kind.into(),
            category,
            actor_kind: actor_kind.to_owned(),
            actor: actor.to_owned(),
            summary: summary.into(),
            subject: None,
            detail: Value::Object(Map::new()),
            coalesce: false,
        }
    }

    /// An entry the server made itself.
    pub fn server(
        kind: impl Into<String>,
        category: &'static str,
        summary: impl Into<String>,
    ) -> NewActivity {
        NewActivity::by(kind, category, "server", "server", summary)
    }

    /// Link the entry to `kind`'s `id`.
    pub fn subject(mut self, kind: &str, id: impl Into<String>) -> NewActivity {
        self.subject = Some((kind.to_owned(), id.into()));
        self
    }

    /// Set the detail, a JSON object.
    pub fn detail(mut self, detail: Value) -> NewActivity {
        self.detail = detail;
        self
    }

    /// Coalesce the entry with a burst of the same (see [`NewActivity::coalesce`]).
    pub fn coalescing(mut self) -> NewActivity {
        self.coalesce = true;
        self
    }
}

/// One recorded entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activity {
    /// A ULID, so ids sort by when the entry was made.
    pub id: String,
    pub at_ms: i64,
    pub kind: String,
    pub category: String,
    pub actor_kind: String,
    pub actor: String,
    pub summary: String,
    pub subject_kind: Option<String>,
    pub subject: Option<String>,
    pub detail: Value,
}

/// Which entries [`State::activity`] answers. Every filter is optional.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActivityFilter {
    pub category: Option<String>,
    /// An actor kind, such as `agent`, or a kind and name, such as `agent:researcher`.
    pub actor: Option<String>,
    /// A kind, such as `ai.tool`, or a dotted prefix of kinds, such as `ai.turn` or `note`.
    pub kind: Option<String>,
    /// Text the summary holds, any case.
    pub q: Option<String>,
    /// Only entries older than this entry id: the `next` of the page before.
    pub before: Option<String>,
    /// At most this many entries, 1 to [`MAX_PAGE`]; [`DEFAULT_PAGE`] when `None`.
    pub limit: Option<usize>,
}

/// One page of entries, newest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityPage {
    pub items: Vec<Activity>,
    /// The `before` of the next page, or `None` on the last.
    pub next: Option<String>,
}

const COLUMNS: &str =
    "id, at_ms, kind, category, actor_kind, actor, summary, subject_kind, subject, detail";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Activity> {
    let detail: String = r.get(9)?;
    Ok(Activity {
        id: r.get(0)?,
        at_ms: r.get(1)?,
        kind: r.get(2)?,
        category: r.get(3)?,
        actor_kind: r.get(4)?,
        actor: r.get(5)?,
        summary: r.get(6)?,
        subject_kind: r.get(7)?,
        subject: r.get(8)?,
        detail: serde_json::from_str(&detail).unwrap_or_else(|_| Value::Object(Map::new())),
    })
}

/// `s` on one line, trimmed, at most `max` characters, ending in `…` when cut.
pub fn one_line(s: &str, max: usize) -> String {
    let flat: String = s
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    if flat.chars().count() <= max {
        return flat;
    }
    let mut cut: String = flat.chars().take(max.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// `detail` as stored: an object of at most [`MAX_DETAIL`] bytes. A larger one has its long
/// strings cut, and failing that becomes `{"truncated": true}`.
fn bounded_detail(detail: Value) -> String {
    let detail = match detail {
        Value::Object(m) => Value::Object(m),
        Value::Null => Value::Object(Map::new()),
        other => {
            let mut m = Map::new();
            m.insert("value".into(), other);
            Value::Object(m)
        }
    };
    let text = detail.to_string();
    if text.len() <= MAX_DETAIL {
        return text;
    }
    let cut = shorten(detail);
    let text = cut.to_string();
    if text.len() <= MAX_DETAIL {
        return text;
    }
    r#"{"truncated":true}"#.to_owned()
}

fn shorten(v: Value) -> Value {
    match v {
        Value::String(s) => Value::String(one_line(&s, 200)),
        Value::Array(a) => Value::Array(a.into_iter().take(20).map(shorten).collect()),
        Value::Object(m) => Value::Object(m.into_iter().map(|(k, v)| (k, shorten(v))).collect()),
        other => other,
    }
}

/// The ULID of an entry made at `now_ms`, after every id `tx` already holds.
fn next_id(tx: &Transaction<'_>, now_ms: i64) -> Result<String> {
    let mut random = [0u8; 10];
    crate::random_bytes(&mut random);
    let random = random
        .iter()
        .fold(0u128, |acc, b| (acc << 8) | u128::from(*b));
    let id = ulid::Ulid::from_parts(u64::try_from(now_ms).unwrap_or(0), random);
    let last: Option<String> = tx
        .query_row(
            "SELECT id FROM activity ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let last = last.and_then(|l| ulid::Ulid::from_string(&l).ok());
    Ok(match last {
        Some(last) if last >= id => last
            .increment()
            .ok_or_else(|| Error::Corrupt("the activity ids have run out".into()))?
            .to_string(),
        _ => id.to_string(),
    })
}

/// `pattern` with SQL `LIKE`'s wildcards escaped by `\`.
fn like_escape(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len());
    for c in pattern.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

impl State {
    /// Record `entry` and publish it. Never fails: an error is logged and the entry dropped, so
    /// the operation that made it goes on. Returns the entry as stored, or `None` on an error.
    pub fn record_activity(&self, entry: NewActivity) -> Option<Activity> {
        match self.try_record_activity(entry) {
            Ok(a) => Some(a),
            Err(e) => {
                eprintln!("activity log: an entry was not recorded: {e}");
                None
            }
        }
    }

    fn try_record_activity(&self, entry: NewActivity) -> Result<Activity> {
        let now = self.now();
        let due = {
            let last = self
                .activity_pruned
                .load(std::sync::atomic::Ordering::Relaxed);
            now.saturating_sub(last) >= PRUNE_EVERY_MS
        };
        if due {
            self.activity_pruned
                .store(now, std::sync::atomic::Ordering::Relaxed);
            if let Err(e) = self.prune_activity() {
                eprintln!("activity log: pruning failed: {e}");
            }
        }
        let summary = one_line(&entry.summary, MAX_SUMMARY);
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let (mut actor_kind, mut actor) = (entry.actor_kind, entry.actor);
        if actor_kind == "token" && actor.starts_with("device-") {
            let name: Option<String> = tx
                .query_row(
                    "SELECT name FROM devices WHERE token_id = ?1",
                    [&actor],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(name) = name {
                actor_kind = "device".to_owned();
                actor = name;
            }
        }
        let (subject_kind, subject) = match entry.subject {
            Some((k, s)) => (Some(k), Some(s)),
            None => (None, None),
        };
        if entry.coalesce
            && let (Some(sk), Some(s)) = (&subject_kind, &subject)
        {
            let newest = tx
                .query_row(
                    &format!(
                        "SELECT {COLUMNS} FROM activity WHERE subject_kind = ?1 AND subject = ?2
                         ORDER BY id DESC LIMIT 1"
                    ),
                    params![sk, s],
                    row,
                )
                .optional()?;
            if let Some(old) = newest
                && old.kind == entry.kind
                && old.actor_kind == actor_kind
                && old.actor == actor
                && now.saturating_sub(old.at_ms) < COALESCE_MS
            {
                let edits = old.detail.get("edits").and_then(Value::as_u64).unwrap_or(1) + 1;
                let mut detail = match old.detail {
                    Value::Object(m) => m,
                    _ => Map::new(),
                };
                if let Value::Object(new) = entry.detail {
                    detail.extend(new);
                }
                detail.insert("edits".into(), Value::from(edits));
                detail.insert("last_ms".into(), Value::from(now));
                let detail_text = bounded_detail(Value::Object(detail));
                tx.execute(
                    "UPDATE activity SET summary = ?2, detail = ?3 WHERE id = ?1",
                    params![old.id, summary, detail_text],
                )?;
                tx.commit()?;
                let item = Activity {
                    summary,
                    detail: serde_json::from_str(&detail_text).unwrap_or(Value::Null),
                    ..old
                };
                self.events
                    .publish(LiveEvent::Activity { item: item.clone() });
                return Ok(item);
            }
        }
        let detail_text = bounded_detail(entry.detail);
        let id = next_id(&tx, now)?;
        tx.execute(
            "INSERT INTO activity
                (id, at_ms, kind, category, actor_kind, actor, summary, subject_kind, subject, detail)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                id,
                now,
                entry.kind,
                entry.category,
                actor_kind,
                actor,
                summary,
                subject_kind,
                subject,
                detail_text
            ],
        )?;
        tx.commit()?;
        let item = Activity {
            id,
            at_ms: now,
            kind: entry.kind,
            category: entry.category.to_owned(),
            actor_kind,
            actor,
            summary,
            subject_kind,
            subject,
            detail: serde_json::from_str(&detail_text).unwrap_or(Value::Null),
        };
        self.events
            .publish(LiveEvent::Activity { item: item.clone() });
        Ok(item)
    }

    /// A page of entries matching `filter`, newest first.
    pub fn activity(&self, filter: &ActivityFilter) -> Result<ActivityPage> {
        let limit = filter.limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);
        let (actor_kind, actor) = match filter.actor.as_deref().filter(|a| !a.is_empty()) {
            Some(a) => match a.split_once(':') {
                Some((k, n)) => (Some(k.to_owned()), Some(n.to_owned())),
                None => (Some(a.to_owned()), None),
            },
            None => (None, None),
        };
        let nonempty = |s: &Option<String>| s.clone().filter(|s| !s.trim().is_empty());
        let kind = nonempty(&filter.kind);
        let kind_prefix = kind.as_deref().map(|k| format!("{}.%", like_escape(k)));
        let q = nonempty(&filter.q).map(|q| format!("%{}%", like_escape(q.trim())));
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM activity
             WHERE (?1 IS NULL OR category = ?1)
               AND (?2 IS NULL OR actor_kind = ?2)
               AND (?3 IS NULL OR actor = ?3)
               AND (?4 IS NULL OR kind = ?4 OR kind LIKE ?5 ESCAPE '\\')
               AND (?6 IS NULL OR summary LIKE ?6 ESCAPE '\\')
               AND (?7 IS NULL OR id < ?7)
             ORDER BY id DESC LIMIT ?8"
        ))?;
        let mut items = stmt
            .query_map(
                params![
                    nonempty(&filter.category),
                    actor_kind,
                    actor,
                    kind,
                    kind_prefix,
                    q,
                    nonempty(&filter.before),
                    limit as i64 + 1
                ],
                row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let next = if items.len() > limit {
            items.truncate(limit);
            items.last().map(|a| a.id.clone())
        } else {
            None
        };
        Ok(ActivityPage { items, next })
    }

    /// Prune the log to [`RETENTION_MS`] and [`MAX_ENTRIES`]. Returns how many entries went.
    pub fn prune_activity(&self) -> Result<usize> {
        self.prune_activity_to(RETENTION_MS, MAX_ENTRIES)
    }

    /// Remove entries older than `max_age_ms` and beyond the newest `max_entries`. Returns how
    /// many went.
    pub fn prune_activity_to(&self, max_age_ms: i64, max_entries: usize) -> Result<usize> {
        let cutoff = self.now().saturating_sub(max_age_ms);
        let conn = self.conn();
        let mut gone = conn.execute("DELETE FROM activity WHERE at_ms < ?1", [cutoff])?;
        let oldest_kept: Option<String> = conn
            .query_row(
                "SELECT id FROM activity ORDER BY id DESC LIMIT 1 OFFSET ?1",
                [max_entries.saturating_sub(1) as i64],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = oldest_kept {
            gone += conn.execute("DELETE FROM activity WHERE id < ?1", [id])?;
        }
        Ok(gone)
    }
}
