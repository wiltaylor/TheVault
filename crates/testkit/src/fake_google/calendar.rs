//! The fake Google Calendar API, under [`super::CALENDAR_PATH`] (google-sync D7, D8, D9, D16).
//!
//! | Request | Answers |
//! |---------|---------|
//! | `GET /users/me/calendarList` | `{ items: [{ id, summary, primary, accessRole }] }` |
//! | `GET /calendars/{cal}/events` | Without `syncToken`: every event that is not cancelled, series masters and changed instances as they are stored (`singleEvents` is not expanded). With `syncToken`: every event changed since the token, cancelled ones included. Paged by `maxResults` (default 250) and `pageToken`; the last page carries `nextSyncToken`. A token from before [`FakeGoogle::expire_sync_tokens`], or one the fake never issued, answers 410 |
//! | `GET /calendars/{cal}/events/{id}/instances` | The occurrences of series `{id}` that overlap `timeMin` to `timeMax`, a stored changed instance in place of the generated one, cancelled ones left out |
//! | `GET /calendars/{cal}/events/{id}` | The event or one occurrence, cancelled ones included with `status = "cancelled"`; 404 for an id it never had |
//! | `POST /calendars/{cal}/events` | Inserts the JSON body as a new event and answers it |
//! | `PATCH /calendars/{cal}/events/{id}` | Sets the body's top-level members. An `If-Match` that is not the current etag answers 412. Patching an occurrence stores it as a changed instance |
//! | `DELETE /calendars/{cal}/events/{id}` | Cancels the event or occurrence, 204; 410 when it is already cancelled |
//!
//! Every request needs an access token the token endpoint issued. The calendar `primary` always
//! exists; [`FakeGoogleBuilder::calendar`] adds others. Each change gives the event a new etag,
//! `"<n>"` with the quotes as Google sends them, and an `updated` time: the event's own when a
//! test stores it with one, else the fake's clock, which [`FakeGoogle::set_calendar_time`] sets.
//!
//! [`FakeGoogle::set_calendar_delay`] holds every request for a while, so a sync run is slow.
//!
//! Series expand from their first `RRULE` with `FREQ` `DAILY` or `WEEKLY`, `INTERVAL`, `COUNT`
//! and `UNTIL`. An occurrence's id is the series id, `_`, and its original start: `YYYYMMDD` for
//! an all-day series, `YYYYMMDDTHHMMSSZ` in UTC for a timed one, as Google's are.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use super::{FakeGoogle, FakeGoogleBuilder, Shared, lock};
use crate::standin::{Reply, Request, url_decode};

/// The fake's clock before a test sets it.
const DEFAULT_TIME: &str = "2026-01-01T00:00:00.000Z";

/// How the fake Calendar starts.
#[derive(Debug, Clone, Default)]
pub(super) struct Config {
    calendars: Vec<(String, String)>,
}

impl FakeGoogleBuilder {
    /// Add calendar `id`, named `summary`, beside `primary`.
    pub fn calendar(mut self, id: &str, summary: &str) -> Self {
        self.calendar
            .calendars
            .push((id.to_owned(), summary.to_owned()));
        self
    }
}

/// One stored event: a single event, a series master or a changed (or cancelled) instance.
#[derive(Debug, Clone)]
struct Stored {
    calendar: String,
    event: Value,
    /// The change counter when it last changed.
    seq: u64,
}

/// The fake Calendar's calendars and events.
#[derive(Debug)]
pub(super) struct State {
    calendars: Vec<(String, String)>,
    /// By calendar id, then event id.
    events: BTreeMap<(String, String), Stored>,
    seq: u64,
    etags: u64,
    ids: u64,
    /// Sync tokens carry this; [`FakeGoogle::expire_sync_tokens`] moves it on.
    generation: u64,
    time: String,
    /// How long each request waits before it is answered, from [`FakeGoogle::set_calendar_delay`].
    delay: Option<std::time::Duration>,
}

impl State {
    pub(super) fn new(config: Config) -> State {
        let mut calendars = vec![("primary".to_owned(), "Primary".to_owned())];
        calendars.extend(config.calendars);
        State {
            calendars,
            events: BTreeMap::new(),
            seq: 0,
            etags: 0,
            ids: 0,
            generation: 1,
            time: DEFAULT_TIME.to_owned(),
            delay: None,
        }
    }

    fn has_calendar(&self, id: &str) -> bool {
        self.calendars.iter().any(|(c, _)| c == id)
    }

    /// Store `event` in `calendar` as changed now: a new etag and change number, and `updated`
    /// set unless `keep_updated` and the event has one.
    fn store(&mut self, calendar: &str, mut event: Value, keep_updated: bool) -> Value {
        self.seq += 1;
        self.etags += 1;
        let obj = event.as_object_mut().expect("an event is a JSON object");
        obj.insert("kind".into(), json!("calendar#event"));
        obj.insert("etag".into(), json!(format!("\"{}\"", self.etags)));
        if !keep_updated || !obj.contains_key("updated") {
            obj.insert("updated".into(), json!(self.time));
        }
        obj.entry("status").or_insert(json!("confirmed"));
        obj.entry("created").or_insert(json!(self.time));
        let id = obj["id"].as_str().expect("an event has an id").to_owned();
        self.events.insert(
            (calendar.to_owned(), id),
            Stored {
                calendar: calendar.to_owned(),
                event: event.clone(),
                seq: self.seq,
            },
        );
        event
    }

    fn get(&self, calendar: &str, id: &str) -> Option<&Value> {
        self.events
            .get(&(calendar.to_owned(), id.to_owned()))
            .map(|s| &s.event)
    }

    /// The event or occurrence `id`: stored, or generated from its series.
    fn find(&self, calendar: &str, id: &str) -> Option<Value> {
        if let Some(e) = self.get(calendar, id) {
            return Some(e.clone());
        }
        let (master_id, _) = id.rsplit_once('_')?;
        let master = self.get(calendar, master_id)?;
        if master["status"] == "cancelled" {
            return None;
        }
        occurrences(master, i64::MIN, i64::MAX)
            .into_iter()
            .find(|o| o["id"] == id)
    }

    fn token(&self) -> String {
        format!("sync_{}_{}", self.generation, self.seq)
    }

    /// The change number a sync token stands for, or `None` when it is not one this fake's
    /// current generation issued.
    fn token_seq(&self, token: &str) -> Option<u64> {
        let rest = token.strip_prefix("sync_")?;
        let (generation, seq) = rest.split_once('_')?;
        (generation.parse::<u64>().ok()? == self.generation)
            .then(|| seq.parse().ok())
            .flatten()
    }
}

impl FakeGoogle {
    /// Store `event`, a Calendar event resource, in `calendar`, replacing any with its `id`.
    /// It gets an id when it has none, a new etag, and `updated` unless it carries one. Returns
    /// the stored event. A series master carries `recurrence`; a changed occurrence carries its
    /// occurrence id, `recurringEventId` and `originalStartTime`.
    ///
    /// # Panics
    ///
    /// When `calendar` does not exist or `event` is not an object.
    pub fn put_event(&self, calendar: &str, mut event: Value) -> Value {
        let mut cal = self.calendar();
        assert!(cal.has_calendar(calendar), "no calendar {calendar}");
        if event.get("id").is_none() {
            cal.ids += 1;
            event["id"] = json!(format!("ev{}", cal.ids));
        }
        cal.store(calendar, event, true)
    }

    /// Change the members `changes` sets on event or occurrence `id`, as a change made in Google.
    ///
    /// # Panics
    ///
    /// When there is no such event.
    pub fn change_event(&self, calendar: &str, id: &str, changes: Value) -> Value {
        let mut cal = self.calendar();
        let mut event = cal
            .find(calendar, id)
            .unwrap_or_else(|| panic!("no event {id} in {calendar}"));
        merge(&mut event, &changes);
        let keep = changes.get("updated").is_some();
        cal.store(calendar, event, keep)
    }

    /// Cancel event or occurrence `id`, as deleting it in Google does.
    pub fn cancel_event(&self, calendar: &str, id: &str) -> Value {
        self.change_event(calendar, id, json!({ "status": "cancelled" }))
    }

    /// Event or occurrence `id` as the fake holds it now.
    pub fn event(&self, calendar: &str, id: &str) -> Option<Value> {
        self.calendar().find(calendar, id)
    }

    /// Every event stored in `calendar`: single events, series masters and changed instances,
    /// cancelled ones included.
    pub fn events(&self, calendar: &str) -> Vec<Value> {
        self.calendar()
            .events
            .values()
            .filter(|s| s.calendar == calendar)
            .map(|s| s.event.clone())
            .collect()
    }

    /// Make every sync token issued so far answer 410, as Google does when a token expires.
    pub fn expire_sync_tokens(&self) {
        self.calendar().generation += 1;
    }

    /// Set the time the fake stamps on `updated` from now on, an RFC 3339 time.
    pub fn set_calendar_time(&self, at: &str) {
        self.calendar().time = at.to_owned();
    }

    /// Wait `delay` before answering each Calendar request, or answer at once with `None`, so a
    /// sync run is slow. A request already waiting is held to the new delay, so `None` releases it.
    pub fn set_calendar_delay(&self, delay: Option<std::time::Duration>) {
        self.calendar().delay = delay;
    }
}

/// Set each member of `changes` on `event`.
fn merge(event: &mut Value, changes: &Value) {
    if let (Some(e), Some(c)) = (event.as_object_mut(), changes.as_object()) {
        for (k, v) in c {
            e.insert(k.clone(), v.clone());
        }
    }
}

/// Answer a Calendar request, or `None` when `request` is not one.
pub(super) fn handle(shared: &Shared, request: &Request) -> Option<Reply> {
    let rest = request.route().strip_prefix(super::CALENDAR_PATH)?;
    if !super::authorized(shared, request) {
        return Some(super::unauthorized());
    }
    // Wait without holding the calendars, so the test can still change them.
    super::hold(|| lock(&shared.calendar).delay);
    let parts: Vec<String> = rest
        .split('/')
        .filter(|p| !p.is_empty())
        .map(url_decode)
        .collect();
    let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
    let mut cal = lock(&shared.calendar);
    let method = request.method.as_str();
    let reply = match parts.as_slice() {
        ["users", "me", "calendarList"] if method == "GET" => {
            let items: Vec<Value> = cal
                .calendars
                .iter()
                .map(|(id, summary)| {
                    json!({
                        "kind": "calendar#calendarListEntry",
                        "id": id,
                        "summary": summary,
                        "primary": id == "primary",
                        "accessRole": "owner",
                    })
                })
                .collect();
            Reply::json(
                200,
                &json!({ "kind": "calendar#calendarList", "items": items }),
            )
        }
        ["calendars", c, ..] if !cal.has_calendar(c) => super::error(404, "NOT_FOUND", "Not Found"),
        ["calendars", c, "events"] => match method {
            "GET" => list(&cal, c, request),
            "POST" => insert(&mut cal, c, request),
            _ => super::error(405, "METHOD_NOT_ALLOWED", "Method not allowed."),
        },
        ["calendars", c, "events", id, "instances"] if method == "GET" => {
            instances(&cal, c, id, request)
        }
        ["calendars", c, "events", id] => match method {
            "GET" => match cal.find(c, id) {
                Some(e) => Reply::json(200, &e),
                None => super::error(404, "NOT_FOUND", "Not Found"),
            },
            "PATCH" => patch(&mut cal, c, id, request),
            "DELETE" => match cal.find(c, id) {
                None => super::error(404, "NOT_FOUND", "Not Found"),
                Some(e) if e["status"] == "cancelled" => {
                    super::error(410, "GONE", "Resource has been deleted")
                }
                Some(mut e) => {
                    e["status"] = json!("cancelled");
                    cal.store(c, e, false);
                    Reply {
                        status: 204,
                        headers: Vec::new(),
                        body: Vec::new(),
                        delay: std::time::Duration::ZERO,
                    }
                }
            },
            _ => super::error(405, "METHOD_NOT_ALLOWED", "Method not allowed."),
        },
        _ => super::error(404, "NOT_FOUND", "Not Found"),
    };
    Some(reply)
}

fn bad_request(message: &str) -> Reply {
    super::error(400, "INVALID_ARGUMENT", message)
}

/// One page of `items` from `offset`, with `extra` members, and the page token of the next page
/// built by `next` from its offset.
fn page(
    items: Vec<Value>,
    request: &Request,
    next: impl Fn(usize) -> String,
    last: Map<String, Value>,
) -> Result<Value, Reply> {
    let size = match request.query("maxResults") {
        None => 250,
        Some(n) => match n.parse::<usize>() {
            Ok(n) if n > 0 => n.min(2500),
            _ => return Err(bad_request("Invalid maxResults.")),
        },
    };
    let offset = match request.query("pageToken") {
        None => 0,
        Some(t) => match t.rsplit('_').next().and_then(|o| o.parse::<usize>().ok()) {
            Some(o) => o,
            None => return Err(bad_request("Invalid pageToken.")),
        },
    };
    let end = (offset + size).min(items.len());
    let mut out = Map::new();
    out.insert("kind".into(), json!("calendar#events"));
    out.insert(
        "items".into(),
        Value::Array(items.get(offset..end).unwrap_or_default().to_vec()),
    );
    if end < items.len() {
        out.insert("nextPageToken".into(), json!(next(end)));
    } else {
        out.extend(last);
    }
    Ok(Value::Object(out))
}

fn list(cal: &State, calendar: &str, request: &Request) -> Reply {
    // A page token carries the change number the listing started from: `p_<since>_<offset>`,
    // with `since` `-` for a full listing.
    let since = match (request.query("pageToken"), request.query("syncToken")) {
        (Some(p), _) => match p.split('_').nth(1) {
            Some("-") => None,
            Some(s) => match s.parse::<u64>() {
                Ok(s) => Some(s),
                Err(_) => return bad_request("Invalid pageToken."),
            },
            None => return bad_request("Invalid pageToken."),
        },
        (None, Some(t)) => match cal.token_seq(&t) {
            Some(s) => Some(s),
            None => {
                return super::error(
                    410,
                    "GONE",
                    "Sync token is no longer valid, a full sync is required.",
                );
            }
        },
        (None, None) => None,
    };
    let items: Vec<Value> = cal
        .events
        .values()
        .filter(|s| s.calendar == calendar)
        .filter(|s| match since {
            Some(since) => s.seq > since,
            None => s.event["status"] != "cancelled",
        })
        .map(|s| s.event.clone())
        .collect();
    let mark = since.map_or("-".to_owned(), |s| s.to_string());
    let mut last = Map::new();
    last.insert("nextSyncToken".into(), json!(cal.token()));
    match page(items, request, |o| format!("p_{mark}_{o}"), last) {
        Ok(body) => Reply::json(200, &body),
        Err(reply) => reply,
    }
}

fn instances(cal: &State, calendar: &str, id: &str, request: &Request) -> Reply {
    let Some(master) = cal.get(calendar, id) else {
        return super::error(404, "NOT_FOUND", "Not Found");
    };
    if master["status"] == "cancelled" {
        return super::error(410, "GONE", "Resource has been deleted");
    }
    let bound = |name: &str, or: i64| match request.query(name) {
        None => Ok(or),
        Some(t) => parse_time(&t)
            .map(|(s, _)| s)
            .ok_or_else(|| bad_request(&format!("Invalid {name}."))),
    };
    let (from, to) = match (bound("timeMin", i64::MIN), bound("timeMax", i64::MAX)) {
        (Ok(f), Ok(t)) => (f, t),
        (Err(r), _) | (_, Err(r)) => return r,
    };
    let items: Vec<Value> = if master.get("recurrence").is_none() {
        vec![master.clone()]
    } else {
        occurrences(master, from, to)
            .into_iter()
            .map(|o| {
                let o_id = o["id"].as_str().unwrap_or_default().to_owned();
                cal.get(calendar, &o_id).cloned().unwrap_or(o)
            })
            .filter(|o| o["status"] != "cancelled")
            .collect()
    };
    match page(items, request, |o| format!("i_{o}"), Map::new()) {
        Ok(body) => Reply::json(200, &body),
        Err(reply) => reply,
    }
}

fn insert(cal: &mut State, calendar: &str, request: &Request) -> Reply {
    let mut event = request.json();
    if !event.is_object() {
        return bad_request("The body must be an event.");
    }
    cal.ids += 1;
    event["id"] = json!(format!("ins{}", cal.ids));
    if let Some(o) = event.as_object_mut() {
        o.remove("etag");
        o.remove("updated");
    }
    Reply::json(200, &cal.store(calendar, event, false))
}

fn patch(cal: &mut State, calendar: &str, id: &str, request: &Request) -> Reply {
    let Some(mut event) = cal.find(calendar, id) else {
        return super::error(404, "NOT_FOUND", "Not Found");
    };
    if let Some(want) = request.header("if-match")
        && want != "*"
        && Some(want) != event["etag"].as_str()
    {
        return super::error(412, "FAILED_PRECONDITION", "Precondition Failed");
    }
    let changes = request.json();
    if !changes.is_object() {
        return bad_request("The body must be an event.");
    }
    merge(&mut event, &changes);
    event["id"] = json!(id);
    Reply::json(200, &cal.store(calendar, event, false))
}

// ---- series expansion ---------------------------------------------------------------------

/// A start or end of an event: an all-day date as days since the epoch, or a time as seconds
/// since the epoch with the offset it was written in.
#[derive(Debug, Clone, Copy)]
enum When {
    Date(i64),
    Time(i64, i64),
}

impl When {
    fn of(v: &Value) -> Option<When> {
        if let Some(d) = v.get("date").and_then(Value::as_str) {
            return parse_date(d).map(When::Date);
        }
        let t = v.get("dateTime").and_then(Value::as_str)?;
        parse_time(t).map(|(s, o)| When::Time(s, o))
    }

    /// Seconds since the epoch, an all-day date taken at midnight UTC.
    fn secs(self) -> i64 {
        match self {
            When::Date(d) => d * 86_400,
            When::Time(s, _) => s,
        }
    }

    fn plus_days(self, days: i64) -> When {
        match self {
            When::Date(d) => When::Date(d + days),
            When::Time(s, o) => When::Time(s + days * 86_400, o),
        }
    }

    fn to_json(self, like: &Value) -> Value {
        let mut out = Map::new();
        match self {
            When::Date(d) => {
                out.insert("date".into(), json!(format_date(d)));
            }
            When::Time(s, o) => {
                out.insert("dateTime".into(), json!(format_time(s, o)));
                if let Some(z) = like.get("timeZone") {
                    out.insert("timeZone".into(), z.clone());
                }
            }
        }
        Value::Object(out)
    }

    /// The part of an occurrence id after the series id.
    fn stamp(self) -> String {
        match self {
            When::Date(d) => format_date(d).replace('-', ""),
            When::Time(s, _) => format_time(s, 0).replace(['-', ':'], ""),
        }
    }
}

/// The occurrences of series `master` that overlap `from` to `to`, in seconds since the epoch.
fn occurrences(master: &Value, from: i64, to: i64) -> Vec<Value> {
    let (Some(start), Some(end)) = (
        master.get("start").and_then(When::of),
        master.get("end").and_then(When::of),
    ) else {
        return Vec::new();
    };
    let Some(rule) = master["recurrence"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .find_map(|r| r.strip_prefix("RRULE:"))
    else {
        return Vec::new();
    };
    let mut step = 0;
    let mut interval = 1;
    let mut count: Option<i64> = None;
    let mut until: Option<i64> = None;
    for part in rule.split(';') {
        let (k, v) = part.split_once('=').unwrap_or((part, ""));
        match k {
            "FREQ" => {
                step = match v {
                    "DAILY" => 1,
                    "WEEKLY" => 7,
                    _ => 0,
                }
            }
            "INTERVAL" => interval = v.parse().unwrap_or(1),
            "COUNT" => count = v.parse().ok(),
            "UNTIL" => until = parse_until(v),
            _ => {}
        }
    }
    if step == 0 {
        return Vec::new();
    }
    let length = end.secs() - start.secs();
    let mut out = Vec::new();
    for n in 0..10_000_i64 {
        if count.is_some_and(|c| n >= c) {
            break;
        }
        let s = start.plus_days(n * step * interval);
        if until.is_some_and(|u| s.secs() > u) || s.secs() >= to {
            break;
        }
        if s.secs() + length <= from {
            continue;
        }
        let e = end.plus_days(n * step * interval);
        let mut o = master.clone();
        let obj = o.as_object_mut().expect("an event is an object");
        obj.remove("recurrence");
        let master_id = master["id"].as_str().unwrap_or_default();
        obj.insert("id".into(), json!(format!("{master_id}_{}", s.stamp())));
        obj.insert("recurringEventId".into(), json!(master_id));
        obj.insert("originalStartTime".into(), s.to_json(&master["start"]));
        obj.insert("start".into(), s.to_json(&master["start"]));
        obj.insert("end".into(), e.to_json(&master["end"]));
        out.push(o);
    }
    out
}

fn parse_until(v: &str) -> Option<i64> {
    let digits = |r: std::ops::Range<usize>| v.get(r).and_then(|s| s.parse::<i64>().ok());
    let day = days_from_civil(digits(0..4)?, digits(4..6)?, digits(6..8)?);
    if v.len() >= 15 {
        Some(day * 86_400 + digits(9..11)? * 3600 + digits(11..13)? * 60 + digits(13..15)?)
    } else {
        Some(day * 86_400 + 86_399)
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (
        if m <= 2 {
            yoe + era * 400 + 1
        } else {
            yoe + era * 400
        },
        m,
        d,
    )
}

fn parse_date(s: &str) -> Option<i64> {
    let num = |r: std::ops::Range<usize>| s.get(r).and_then(|p| p.parse::<i64>().ok());
    if s.len() != 10 {
        return None;
    }
    Some(days_from_civil(num(0..4)?, num(5..7)?, num(8..10)?))
}

fn format_date(days: i64) -> String {
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `YYYY-MM-DDTHH:MM:SS[.fff](Z|±HH:MM)` as seconds since the epoch and the offset in seconds.
fn parse_time(s: &str) -> Option<(i64, i64)> {
    let num = |r: std::ops::Range<usize>| s.get(r).and_then(|p| p.parse::<i64>().ok());
    let day = parse_date(s.get(0..10)?)?;
    if s.as_bytes().get(10) != Some(&b'T') {
        return None;
    }
    let local = day * 86_400 + num(11..13)? * 3600 + num(14..16)? * 60 + num(17..19)?;
    let mut zone = s.get(19..)?;
    if let Some(frac) = zone.strip_prefix('.') {
        zone = frac.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    let offset = if zone == "Z" {
        0
    } else {
        let sign = match zone.as_bytes().first()? {
            b'+' => 1,
            b'-' => -1,
            _ => return None,
        };
        let h: i64 = zone.get(1..3)?.parse().ok()?;
        let m: i64 = zone.get(4..6)?.parse().ok()?;
        sign * (h * 3600 + m * 60)
    };
    Some((local - offset, offset))
}

/// Seconds since the epoch as RFC 3339 in `offset`, with `Z` for UTC.
fn format_time(secs: i64, offset: i64) -> String {
    let local = secs + offset;
    let (y, m, d) = civil_from_days(local.div_euclid(86_400));
    let t = local.rem_euclid(86_400);
    let zone = if offset == 0 {
        "Z".to_owned()
    } else {
        let sign = if offset < 0 { '-' } else { '+' };
        let o = offset.abs();
        format!("{sign}{:02}:{:02}", o / 3600, o % 3600 / 60)
    };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}{zone}",
        t / 3600,
        t % 3600 / 60,
        t % 60
    )
}
