//! Step 6, built in: event series and their occurrences (event-series E1, E4, E5).
//!
//! An `event_series` note holds a recurring event: its `rule`, RFC 5545 lines, and `starts`, the
//! first occurrence. Each occurrence is an `event` note whose `series_ref` names the series. A
//! series with a Google `calendar` gets its occurrences from the calendar sync; a series with no
//! calendar and no `google_ref` is kept in the vault only, and this step makes its occurrences.
//!
//! On every writer, a series' rule is checked: each line must be an `RRULE`, `EXDATE` or `RDATE`
//! line that parses, and at least one must be an `RRULE` or `RDATE` line.
//!
//! On the home server only, so that a change made on a device is worked out once, when it
//! arrives there, the step also does the following in the same commit as the change:
//!
//! - **Carried fields** (E4). A field that `event` and `event_series` both declare with the same
//!   name and kind, that is not computed, and that is not a field of either shipped type, is
//!   carried: a change to it on a series, a new series, and an occurrence newly linked to a
//!   series give each occurrence the series' value, except an occurrence whose own value was
//!   changed by hand. The occurrence's hidden, computed `series_overrides` names those fields.
//!   Changing a carried field on an occurrence to a value other than the series' adds the field
//!   there; changing it to the series' value, or clearing it, takes it out again, and a cleared
//!   field takes the series' value back.
//! - **Occurrences of a vault-only series** (E5), from 30 days before today, or the series'
//!   first day when that is later, to a year ahead, today in the series' time zone. Each is an
//!   `event` note with the series' title, location, description and carried fields, and
//!   `series_key`, `<series id>_<original start>`, so the step finds it again. A change to the
//!   rule or the first occurrence makes the occurrences again: a new one is created, one the rule
//!   no longer produces moves to the trash unless it was changed by hand (its `series_overrides`,
//!   time or title differ from what the series made), in which case it stays as it is, still
//!   linked. An occurrence still produced takes the series' new title, location, description and
//!   time where it still holds what the series gave it. [`Writer::roll_series`] makes the new
//!   days' occurrences once a day.
//! - Deleting an occurrence of a vault-only series by hand adds an `EXDATE` line for it to the
//!   series' rule, so it is not made again. Deleting a series deletes its occurrences: `series_ref`
//!   is `on_delete = "cascade"`.
//!
//! Every change the step adds is authored `task server`.

use std::collections::HashSet;
use std::str::FromStr;
use std::sync::OnceLock;

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use schema::{Doc, DocBlock, FieldDef, Invalid, Issue, Schema, TypeDef, Value};
use state::Author;

use crate::builtin::{
    DEFAULT_EVENT_SERIES_TYPE, DEFAULT_EVENT_TYPE, EVENT_SERIES_TYPE, EVENT_TYPE,
};
use crate::{Error, Event, Inner, Input, NoteChange, Result, Writer, note, note_path, time};

/// How many days before today a vault-only series has occurrences from.
pub const DAYS_BEHIND: i64 = 30;

/// How many days after today a vault-only series has occurrences until.
pub const DAYS_AHEAD: i64 = 365;

/// The most occurrences one series makes in its window.
const MAX_OCCURRENCES: u16 = 2000;

/// What the step did across the rounds of one write.
#[derive(Debug, Default)]
pub(crate) struct Pass {
    /// The notes whose changes the step added, which are not edits by hand.
    pub(crate) made: HashSet<String>,
    /// The series whose carried fields and occurrences the write has worked out.
    done: HashSet<String>,
}

/// One occurrence a series' rule produces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occurrence {
    /// The original start: `YYYYMMDD` for an all-day series, `YYYYMMDDTHHMMSSZ` in UTC for a
    /// timed one, as Google's occurrence ids end.
    pub key: String,
    pub start: String,
    pub end: String,
}

/// Whether `doc`, a series, is kept in the vault only: it has no calendar and no Google id.
pub(crate) fn is_local(doc: &Doc) -> bool {
    text(doc, "calendar").trim().is_empty() && text(doc, "google_ref").is_empty()
}

fn text<'a>(doc: &'a Doc, name: &str) -> &'a str {
    doc.field(name).and_then(Value::as_str).unwrap_or("")
}

/// The rule's lines, without blank ones.
fn rule_lines(doc: &Doc) -> Vec<String> {
    match doc.field("rule") {
        Some(Value::List(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .collect(),
        Some(Value::Str(s)) => s
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

/// A content line's parameters, each `(upper-cased name, value)`.
type Params = Vec<(String, String)>;

/// Split a content line into its upper-cased name, its parameters and its value.
fn split_line(line: &str) -> Option<(String, Params, &str)> {
    let (head, value) = line.split_once(':')?;
    let mut parts = head.split(';');
    let name = parts.next()?.trim().to_ascii_uppercase();
    let mut params = Vec::new();
    for p in parts {
        let (k, v) = p.split_once('=')?;
        params.push((k.trim().to_ascii_uppercase(), v.trim().to_owned()));
    }
    Some((name, params, value.trim()))
}

/// Check the lines of a series' rule, as [`check_rule`] does, without a series to expand them
/// for. `Err` says what is wrong with the first bad line.
pub fn check_rule_lines(lines: &[String]) -> std::result::Result<(), String> {
    let mut produces = false;
    for (i, line) in lines.iter().enumerate() {
        let n = i + 1;
        let bad = |why: &str| format!("line {n}, {line:?}, {why}");
        let Some((name, params, value)) = split_line(line) else {
            return Err(bad("is not an RRULE, EXDATE or RDATE line"));
        };
        match name.as_str() {
            "RRULE" => {
                if !params.is_empty() {
                    return Err(bad("has parameters, which an RRULE line does not take"));
                }
                let value = until_in_utc(value, &Zone::utc_all_day());
                rrule::RRule::<rrule::Unvalidated>::from_str(&value)
                    .map_err(|e| bad(&format!("is not a recurrence rule: {e}")))?;
                produces = true;
            }
            "EXDATE" | "RDATE" => {
                let zone = Zone::utc_all_day();
                let tzid = tzid(&params).map_err(|e| bad(&e))?;
                for v in value.split(',') {
                    zone.instant(v.trim(), tzid)
                        .map_err(|e| bad(&format!("has a date that does not read: {e}")))?;
                }
                if name == "RDATE" {
                    produces = true;
                }
            }
            _ => return Err(bad("is not an RRULE, EXDATE or RDATE line")),
        }
    }
    if !produces {
        return Err("needs an RRULE or RDATE line".into());
    }
    Ok(())
}

/// The issues with series `doc`'s rule, for the write to refuse it.
fn check_rule(doc: &Doc) -> Vec<Issue> {
    match check_rule_lines(&rule_lines(doc)) {
        Ok(()) => Vec::new(),
        Err(e) => vec![Issue::new("rule", e)],
    }
}

/// The `TZID` parameter, as a zone.
fn tzid(params: &[(String, String)]) -> std::result::Result<Option<chrono_tz::Tz>, String> {
    match params.iter().find(|(k, _)| k == "TZID") {
        Some((_, v)) => chrono_tz::Tz::from_str(v)
            .map(Some)
            .map_err(|_| format!("names time zone {v:?}, which there is no such zone")),
        None => Ok(None),
    }
}

/// How a series' times read: the zone its rule repeats in, and whether it is all day.
struct Zone {
    tz: chrono_tz::Tz,
    all_day: bool,
    /// The first occurrence's time of day in `tz`, for a date that stands for an occurrence of a
    /// timed series.
    at: NaiveTime,
}

impl Zone {
    fn utc_all_day() -> Zone {
        Zone {
            tz: chrono_tz::UTC,
            all_day: true,
            at: NaiveTime::MIN,
        }
    }

    /// An iCalendar date or date-time, `YYYYMMDD`, `YYYYMMDDTHHMMSS` or `YYYYMMDDTHHMMSSZ`, as an
    /// instant. A local time is in `tzid`, else in the series' zone; a date is midnight UTC for
    /// an all-day series and the first occurrence's time of day for a timed one.
    fn instant(
        &self,
        v: &str,
        tzid: Option<chrono_tz::Tz>,
    ) -> std::result::Result<DateTime<Utc>, String> {
        let tz = tzid.unwrap_or(self.tz);
        if let Some(utc) = v.strip_suffix('Z') {
            let t = NaiveDateTime::parse_from_str(utc, "%Y%m%dT%H%M%S")
                .map_err(|_| format!("{v:?} is not a date-time"))?;
            return Ok(Utc.from_utc_datetime(&t));
        }
        if v.len() == 8 {
            let d = NaiveDate::parse_from_str(v, "%Y%m%d")
                .map_err(|_| format!("{v:?} is not a date"))?;
            if self.all_day {
                return Ok(Utc.from_utc_datetime(&d.and_time(NaiveTime::MIN)));
            }
            return local(tz, d.and_time(self.at)).ok_or_else(|| format!("{v:?} is not a time"));
        }
        let t = NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%S")
            .map_err(|_| format!("{v:?} is not a date or date-time"))?;
        local(tz, t).ok_or_else(|| format!("{v:?} is not a time in {}", tz.name()))
    }
}

/// Local time `t` in `tz`, the earlier one when a clock change makes it ambiguous, `None` when
/// a clock change skips it.
fn local(tz: chrono_tz::Tz, t: NaiveDateTime) -> Option<DateTime<Utc>> {
    tz.from_local_datetime(&t)
        .earliest()
        .map(|t| t.with_timezone(&Utc))
}

/// An RRULE value with its `UNTIL` in UTC, as the rule library needs it with a zoned start: a
/// date is the end of that day and a local time is in the series' zone.
fn until_in_utc(value: &str, zone: &Zone) -> String {
    value
        .split(';')
        .map(|part| {
            let Some((k, v)) = part.split_once('=') else {
                return part.to_owned();
            };
            if !k.trim().eq_ignore_ascii_case("UNTIL") || v.ends_with('Z') {
                return part.to_owned();
            }
            let end = if v.len() == 8 {
                NaiveDate::parse_from_str(v, "%Y%m%d").ok().and_then(|d| {
                    let t = d.and_hms_opt(23, 59, 59)?;
                    if zone.all_day {
                        Some(Utc.from_utc_datetime(&t))
                    } else {
                        local(zone.tz, t)
                    }
                })
            } else {
                NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%S")
                    .ok()
                    .and_then(|t| local(zone.tz, t))
            };
            match end {
                Some(t) => format!("{k}={}", t.format("%Y%m%dT%H%M%SZ")),
                None => part.to_owned(),
            }
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// The start and end of a series' first occurrence.
enum First {
    /// All day: the first day and how many days after it the last one is.
    Days(NaiveDate, i64),
    /// Timed: the start and how long each occurrence lasts, with the offset to write times in
    /// when the series names no zone.
    Times(DateTime<chrono::FixedOffset>, Duration),
}

fn first_of(doc: &Doc) -> std::result::Result<First, String> {
    let (start, end) = match doc.field("starts") {
        Some(Value::Record(members)) => {
            let get = |k: &str| {
                members
                    .iter()
                    .find(|(n, _)| n == k)
                    .and_then(|(_, v)| v.as_str())
                    .unwrap_or("")
                    .to_owned()
            };
            (get("start"), get("end"))
        }
        _ => return Err("the series has no first occurrence".into()),
    };
    let end = if end.is_empty() { start.clone() } else { end };
    if start.len() == 10 {
        let s = NaiveDate::parse_from_str(&start, "%Y-%m-%d")
            .map_err(|_| format!("{start:?} is not a date"))?;
        let e = NaiveDate::parse_from_str(&end, "%Y-%m-%d").unwrap_or(s);
        return Ok(First::Days(s, (e - s).num_days().max(0)));
    }
    let s = DateTime::parse_from_rfc3339(&start).map_err(|_| format!("{start:?} is not a time"))?;
    let e = DateTime::parse_from_rfc3339(&end).unwrap_or(s);
    Ok(First::Times(s, (e - s).max(Duration::zero())))
}

/// The series' zone: its `timezone`, else UTC.
fn zone_of(doc: &Doc) -> Option<chrono_tz::Tz> {
    chrono_tz::Tz::from_str(text(doc, "timezone").trim()).ok()
}

/// Today in the series' zone, at `now_ms`.
fn today(doc: &Doc, now_ms: i64) -> NaiveDate {
    let now = Utc
        .timestamp_millis_opt(now_ms)
        .single()
        .unwrap_or_default();
    match zone_of(doc) {
        Some(tz) => now.with_timezone(&tz).date_naive(),
        None => now.date_naive(),
    }
}

/// The occurrences series `doc` has whose start falls on a day from `from` to before `to`, in
/// the series' zone, oldest first.
pub fn occurrences(
    doc: &Doc,
    from: NaiveDate,
    to: NaiveDate,
) -> std::result::Result<Vec<Occurrence>, String> {
    let first = first_of(doc)?;
    let named = zone_of(doc);
    let zone = match &first {
        First::Days(..) => Zone::utc_all_day(),
        First::Times(s, _) => {
            let tz = named.unwrap_or(chrono_tz::UTC);
            Zone {
                tz,
                all_day: false,
                at: s.with_timezone(&tz).time(),
            }
        }
    };
    let rtz = rrule::Tz::Tz(zone.tz);
    let start: DateTime<Utc> = match &first {
        First::Days(d, _) => Utc.from_utc_datetime(&d.and_time(NaiveTime::MIN)),
        First::Times(s, _) => s.with_timezone(&Utc),
    };
    let dt_start = start.with_timezone(&rtz);
    let mut set = rrule::RRuleSet::new(dt_start);
    for line in rule_lines(doc) {
        let Some((name, params, value)) = split_line(&line) else {
            return Err(format!("{line:?} is not an RRULE, EXDATE or RDATE line"));
        };
        match name.as_str() {
            "RRULE" => {
                let rule =
                    rrule::RRule::<rrule::Unvalidated>::from_str(&until_in_utc(value, &zone))
                        .map_err(|e| format!("{line:?} is not a recurrence rule: {e}"))?
                        .validate(dt_start)
                        .map_err(|e| format!("{line:?} does not fit the first occurrence: {e}"))?;
                set = set.rrule(rule);
            }
            "EXDATE" | "RDATE" => {
                let tzid = tzid(&params)?;
                for v in value.split(',') {
                    let t = zone.instant(v.trim(), tzid)?.with_timezone(&rtz);
                    set = if name == "EXDATE" {
                        set.exdate(t)
                    } else {
                        set.rdate(t)
                    };
                }
            }
            _ => return Err(format!("{line:?} is not an RRULE, EXDATE or RDATE line")),
        }
    }
    let day_start = |d: NaiveDate| -> DateTime<Utc> {
        if zone.all_day {
            Utc.from_utc_datetime(&d.and_time(NaiveTime::MIN))
        } else {
            local(zone.tz, d.and_time(NaiveTime::MIN))
                .unwrap_or_else(|| Utc.from_utc_datetime(&d.and_time(NaiveTime::MIN)))
        }
    };
    let lo = day_start(from);
    let hi = day_start(to);
    let found = set
        .after((lo - Duration::days(1)).with_timezone(&rtz))
        .before((hi + Duration::days(1)).with_timezone(&rtz))
        .all(MAX_OCCURRENCES);
    let mut out = Vec::new();
    for t in found.dates {
        let t = t.with_timezone(&Utc);
        if t < lo || t >= hi {
            continue;
        }
        out.push(match &first {
            First::Days(_, days) => {
                let d = t.date_naive();
                Occurrence {
                    key: d.format("%Y%m%d").to_string(),
                    start: d.format("%Y-%m-%d").to_string(),
                    end: (d + Duration::days(*days)).format("%Y-%m-%d").to_string(),
                }
            }
            First::Times(s, length) => {
                let write = |t: DateTime<Utc>| match named {
                    Some(tz) => t
                        .with_timezone(&tz)
                        .format("%Y-%m-%dT%H:%M:%S%:z")
                        .to_string(),
                    None => t
                        .with_timezone(s.offset())
                        .format("%Y-%m-%dT%H:%M:%S%:z")
                        .to_string(),
                };
                Occurrence {
                    key: t.format("%Y%m%dT%H%M%SZ").to_string(),
                    start: write(t),
                    end: write(t + *length),
                }
            }
        });
    }
    out.sort_by(|a, b| a.key.cmp(&b.key));
    out.dedup_by(|a, b| a.key == b.key);
    Ok(out)
}

/// The `EXDATE` line that leaves out the occurrence whose original start is `key`.
fn exdate_line(key: &str) -> String {
    if key.len() == 8 {
        format!("EXDATE;VALUE=DATE:{key}")
    } else {
        format!("EXDATE:{key}")
    }
}

/// A span of time two `when` values are compared by: days, or instants in milliseconds.
#[derive(Debug, PartialEq, Eq)]
enum Span {
    Days(NaiveDate, NaiveDate),
    Times(i64, i64),
}

fn span_of(start: &str, end: &str) -> Option<Span> {
    if start.len() == 10 {
        let s = NaiveDate::parse_from_str(start, "%Y-%m-%d").ok()?;
        let e = NaiveDate::parse_from_str(end, "%Y-%m-%d").unwrap_or(s);
        return Some(Span::Days(s, e));
    }
    let s = DateTime::parse_from_rfc3339(start).ok()?.timestamp_millis();
    let e = DateTime::parse_from_rfc3339(end)
        .map(|t| t.timestamp_millis())
        .unwrap_or(s);
    Some(Span::Times(s, e))
}

fn when_of(doc: &Doc) -> Option<Span> {
    let Some(Value::Record(members)) = doc.field("when") else {
        return None;
    };
    let get = |k: &str| {
        members
            .iter()
            .find(|(n, _)| n == k)
            .and_then(|(_, v)| v.as_str())
            .unwrap_or("")
    };
    span_of(get("start"), get("end"))
}

/// The span series `doc` gives the occurrence whose original start is `key`.
fn made_span(doc: &Doc, key: &str) -> Option<Span> {
    match first_of(doc).ok()? {
        First::Days(_, days) => {
            let d = NaiveDate::parse_from_str(key, "%Y%m%d").ok()?;
            Some(Span::Days(d, d + Duration::days(days)))
        }
        First::Times(_, length) => {
            let t = NaiveDateTime::parse_from_str(key.strip_suffix('Z')?, "%Y%m%dT%H%M%S").ok()?;
            let s = Utc.from_utc_datetime(&t).timestamp_millis();
            Some(Span::Times(s, s + length.num_milliseconds()))
        }
    }
}

/// The day an original start `key` falls on, in UTC, near enough to compare with the window.
fn key_day(key: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(key.get(..8)?, "%Y%m%d").ok()
}

/// The value a note holds for field `name`: its item, and its blocks of that name.
#[derive(Debug, Clone, PartialEq, Default)]
struct Held {
    value: Option<Value>,
    blocks: Vec<DocBlock>,
}

impl Held {
    fn is_empty(&self) -> bool {
        self.blocks.is_empty()
            && match &self.value {
                None => true,
                Some(Value::Str(s)) => s.is_empty(),
                Some(Value::List(l)) => l.is_empty(),
                Some(Value::Record(r)) => r.is_empty(),
                _ => false,
            }
    }
}

fn held(doc: &Doc, name: &str) -> Held {
    Held {
        value: doc.field(name).cloned(),
        blocks: doc
            .blocks
            .iter()
            .filter(|b| b.label() == Some(name))
            .cloned()
            .collect(),
    }
}

fn put(doc: &mut Doc, name: &str, h: &Held) {
    doc.fields.retain(|(n, _)| n != name);
    doc.blocks.retain(|b| b.label() != Some(name));
    if h.is_empty() {
        return;
    }
    if let Some(v) = &h.value {
        doc.fields.push((name.to_owned(), v.clone()));
    }
    doc.blocks.extend(h.blocks.iter().cloned());
}

fn set_text(doc: &mut Doc, name: &str, value: &str) {
    let h = Held {
        value: Some(Value::Str(value.to_owned())),
        blocks: Vec::new(),
    };
    put(doc, name, &h);
}

fn overrides(doc: &Doc) -> Vec<String> {
    match doc.field("series_overrides") {
        Some(Value::List(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

fn set_overrides(doc: &mut Doc, names: &[String]) {
    let h = Held {
        value: Some(Value::List(
            names.iter().map(|n| Value::Str(n.clone())).collect(),
        )),
        blocks: Vec::new(),
    };
    put(doc, "series_overrides", &h);
}

/// The names of the fields of the shipped `event` and `event_series` types, which are never
/// carried.
fn shipped_names() -> &'static HashSet<String> {
    static NAMES: OnceLock<HashSet<String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        let mut names: HashSet<String> = ["title", "created", "updated", note::ARCHIVED]
            .into_iter()
            .map(str::to_owned)
            .collect();
        for (name, source) in [
            (EVENT_TYPE, DEFAULT_EVENT_TYPE),
            (EVENT_SERIES_TYPE, DEFAULT_EVENT_SERIES_TYPE),
        ] {
            if let Ok(ty) = TypeDef::parse(name, source) {
                names.extend(ty.fields.iter().map(|f| f.name.clone()));
            }
        }
        names
    })
}

/// The carried fields: those `event` and `event_series` both declare with the same name and
/// kind, not computed, and not a field of either shipped type (E4).
pub fn carried_fields(schema: &Schema) -> Vec<FieldDef> {
    let (Some(event), Some(series)) = (
        schema.type_def(EVENT_TYPE),
        schema.type_def(EVENT_SERIES_TYPE),
    ) else {
        return Vec::new();
    };
    let shipped = shipped_names();
    event
        .fields
        .iter()
        .filter(|f| !f.computed && !f.implied && !shipped.contains(&f.name))
        .filter(|f| {
            series
                .field(&f.name)
                .is_some_and(|s| !s.computed && s.kind == f.kind && s.multiple == f.multiple)
        })
        .cloned()
        .collect()
}

/// The series step of one round of step 6: see the module documentation.
pub(crate) fn expand(
    writer: &Writer,
    inner: &Inner,
    mut changes: Vec<NoteChange>,
    pass: &mut Pass,
) -> Result<Vec<NoteChange>> {
    for c in &changes {
        if c.type_name == EVENT_SERIES_TYPE
            && let Some(doc) = &c.doc
        {
            let issues = check_rule(doc);
            if !issues.is_empty() {
                return Err(Error::Invalid(Invalid {
                    file: note_path(&c.type_name, &c.id),
                    issues,
                }));
            }
        }
    }
    if !writer.is_home() {
        return Ok(changes);
    }
    let carried = carried_fields(&inner.schema);
    let mut step = Step {
        writer,
        inner,
        carried,
        pass,
    };
    // Edits by hand of the occurrences first, so a series change sees what they hold now.
    let mut i = 0;
    while i < changes.len() {
        if changes[i].type_name == EVENT_TYPE && !step.pass.made.contains(&changes[i].id) {
            step.occurrence(&mut changes, i)?;
        }
        i += 1;
    }
    let mut i = 0;
    while i < changes.len() {
        let c = &changes[i];
        if c.type_name == EVENT_SERIES_TYPE
            && c.doc.is_some()
            && step.pass.done.insert(c.id.clone())
        {
            step.series(&mut changes, i)?;
        }
        i += 1;
    }
    Ok(changes)
}

struct Step<'a> {
    writer: &'a Writer,
    inner: &'a Inner,
    carried: Vec<FieldDef>,
    pass: &'a mut Pass,
}

impl Step<'_> {
    /// The note on disk, parsed, or `None` when there is no such file.
    fn load(&self, id: &str, type_name: &str) -> Result<Option<Doc>> {
        let path = note_path(type_name, id);
        if !self.writer.vault.root().join(&path).is_file() {
            return Ok(None);
        }
        let text = self.writer.vault.read_text(&path)?;
        Ok(Some(Input::Wcl(text).parse(&path)?))
    }

    /// Series `id` as the write leaves it: `None` when it is deleted or does not exist.
    fn series_doc(&self, changes: &[NoteChange], id: &str) -> Result<Option<Doc>> {
        if let Some(c) = changes.iter().find(|c| c.id == id) {
            if c.type_name != EVENT_SERIES_TYPE {
                return Ok(None);
            }
            return Ok(c.doc.clone());
        }
        self.load(id, EVENT_SERIES_TYPE)
    }

    /// `doc` in canonical order, keeping its own `created` and `updated`.
    fn arranged(&self, type_name: &str, doc: Doc) -> Result<Doc> {
        let ty = self
            .inner
            .schema
            .type_def(type_name)
            .ok_or_else(|| Error::UnknownType(type_name.to_owned()))?;
        let now = time::utc(self.writer.clock.now_ms());
        let created = crate::created_of(&doc).unwrap_or_else(|| now.clone());
        let updated = doc
            .field("updated")
            .and_then(Value::as_str)
            .map_or(now, str::to_owned);
        Ok(note::arrange(ty, doc, &created, &updated))
    }

    /// Put `doc` as the change to note `id` of `type_name`: replace the write's change to it, or
    /// add one.
    fn put_change(
        &mut self,
        changes: &mut Vec<NoteChange>,
        id: &str,
        type_name: &str,
        doc: Option<Doc>,
        exists: bool,
    ) -> Result<()> {
        let ty = self
            .inner
            .schema
            .type_def(type_name)
            .ok_or_else(|| Error::UnknownType(type_name.to_owned()))?;
        let now = time::utc(self.writer.clock.now_ms());
        let doc = doc.map(|d| {
            let created = crate::created_of(&d).unwrap_or_else(|| now.clone());
            note::arrange(ty, d, &created, &now)
        });
        let event = match (&doc, exists) {
            (None, _) => Event::Delete,
            (Some(_), true) => Event::Update,
            (Some(_), false) => Event::Create,
        };
        match changes.iter_mut().find(|c| c.id == id) {
            Some(c) => {
                if doc.is_none() {
                    c.event = Event::Delete;
                }
                c.doc = doc;
            }
            None => {
                self.pass.made.insert(id.to_owned());
                changes.push(NoteChange {
                    id: id.to_owned(),
                    type_name: type_name.to_owned(),
                    event,
                    doc,
                });
            }
        }
        Ok(())
    }

    /// Track the carried fields changed by hand on occurrence `changes[i]`, or add an EXDATE
    /// for the deleted occurrence of a vault-only series.
    fn occurrence(&mut self, changes: &mut Vec<NoteChange>, i: usize) -> Result<()> {
        let id = changes[i].id.clone();
        let old = match changes[i].event {
            Event::Create => None,
            _ => self.load(&id, EVENT_TYPE)?,
        };
        let Some(doc) = changes[i].doc.clone() else {
            return self.deleted(changes, old.as_ref());
        };
        let series_id = text(&doc, "series_ref").to_owned();
        if series_id.is_empty() || self.carried.is_empty() {
            return Ok(());
        }
        let Some(series) = self.series_doc(changes, &series_id)? else {
            return Ok(());
        };
        let newly = old
            .as_ref()
            .is_none_or(|o| text(o, "series_ref") != series_id);
        let mut next = doc.clone();
        let before = overrides(&doc);
        let mut names = before.clone();
        for f in &self.carried {
            let mine = held(&doc, &f.name);
            let theirs = held(&series, &f.name);
            let was = old.as_ref().map(|o| held(o, &f.name)).unwrap_or_default();
            if !newly && mine == was {
                continue;
            }
            let own = !mine.is_empty() && mine != theirs;
            if mine.is_empty() && !theirs.is_empty() {
                put(&mut next, &f.name, &theirs);
            }
            names.retain(|n| n != &f.name);
            if own {
                names.push(f.name.clone());
            }
        }
        if names != before {
            set_overrides(&mut next, &names);
        }
        if next != doc {
            changes[i].doc = Some(self.arranged(EVENT_TYPE, next)?);
        }
        Ok(())
    }

    /// The occurrence `old` was deleted by hand: when it was made from a vault-only series that
    /// stays, the series' rule gains an EXDATE for it.
    fn deleted(&mut self, changes: &mut Vec<NoteChange>, old: Option<&Doc>) -> Result<()> {
        let Some(old) = old else {
            return Ok(());
        };
        let series_id = text(old, "series_ref").to_owned();
        let Some(key) = text(old, "series_key")
            .strip_prefix(&format!("{series_id}_"))
            .map(str::to_owned)
        else {
            return Ok(());
        };
        let Some(mut series) = self.series_doc(changes, &series_id)? else {
            return Ok(());
        };
        if !is_local(&series) {
            return Ok(());
        }
        let line = exdate_line(&key);
        let mut lines = rule_lines(&series);
        if lines.contains(&line) {
            return Ok(());
        }
        lines.push(line);
        series.fields.retain(|(n, _)| n != "rule");
        series.fields.push((
            "rule".to_owned(),
            Value::List(lines.into_iter().map(Value::Str).collect()),
        ));
        self.put_change(changes, &series_id, EVENT_SERIES_TYPE, Some(series), true)
    }

    /// The occurrences linked to series `id`, as the write leaves them, by id.
    fn linked(&self, changes: &[NoteChange], id: &str) -> Result<Vec<(String, Doc)>> {
        let conn = self.inner.index.connection();
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT src FROM links WHERE dst = ?1 AND via = 'field' \
                 AND field = 'series_ref' ORDER BY src",
            )
            .map_err(index::Error::from)?;
        let rows = stmt
            .query_map([id], |r| r.get::<_, String>(0))
            .map_err(index::Error::from)?;
        let mut ids = Vec::new();
        for row in rows {
            ids.push(row.map_err(index::Error::from)?);
        }
        for c in changes {
            if c.type_name == EVENT_TYPE
                && c.doc.as_ref().is_some_and(|d| text(d, "series_ref") == id)
                && !ids.contains(&c.id)
            {
                ids.push(c.id.clone());
            }
        }
        let mut out = Vec::new();
        for o in ids {
            let doc = match changes.iter().find(|c| c.id == o) {
                Some(c) => c.doc.clone(),
                None => self.load(&o, EVENT_TYPE)?,
            };
            if let Some(doc) = doc.filter(|d| text(d, "series_ref") == id) {
                out.push((o, doc));
            }
        }
        Ok(out)
    }

    /// Carry series `changes[i]`'s carried fields to its occurrences, and make the occurrences of
    /// a vault-only series.
    fn series(&mut self, changes: &mut Vec<NoteChange>, i: usize) -> Result<()> {
        let id = changes[i].id.clone();
        let Some(doc) = changes[i].doc.clone() else {
            return Ok(());
        };
        let old = match changes[i].event {
            Event::Create => None,
            _ => self.load(&id, EVENT_SERIES_TYPE)?,
        };
        let changed: Vec<FieldDef> = self
            .carried
            .iter()
            .filter(|f| {
                let now = held(&doc, &f.name);
                match &old {
                    None => !now.is_empty(),
                    Some(o) => held(o, &f.name) != now,
                }
            })
            .cloned()
            .collect();
        if !changed.is_empty() {
            for (o, occ) in self.linked(changes, &id)? {
                let skip = overrides(&occ);
                let mut next = occ.clone();
                for f in changed.iter().filter(|f| !skip.contains(&f.name)) {
                    put(&mut next, &f.name, &held(&doc, &f.name));
                }
                if next != occ {
                    self.put_change(changes, &o, EVENT_TYPE, Some(next), true)?;
                }
            }
        }
        self.generate(changes, &id, old.as_ref(), &doc)
    }

    /// Make the occurrences of series `id`, `doc` now and `old` before the write, as the module
    /// documentation says.
    fn generate(
        &mut self,
        changes: &mut Vec<NoteChange>,
        id: &str,
        old: Option<&Doc>,
        doc: &Doc,
    ) -> Result<()> {
        let mine: Vec<(String, String, Doc)> = self
            .linked(changes, id)?
            .into_iter()
            .filter_map(|(o, d)| {
                let key = text(&d, "series_key")
                    .strip_prefix(&format!("{id}_"))?
                    .to_owned();
                Some((o, key, d))
            })
            .collect();
        if !is_local(doc) {
            // Given a calendar, the series is Google's: its occurrences come from the calendar
            // sync, so the ones made here go.
            for (o, _, d) in mine {
                if text(&d, "google_ref").is_empty() {
                    self.put_change(changes, &o, EVENT_TYPE, None, true)?;
                }
            }
            return Ok(());
        }
        if note::is_archived(doc) {
            return Ok(());
        }
        let today = today(doc, self.writer.clock.now_ms());
        let mut from = today - Duration::days(DAYS_BEHIND);
        if let Ok(first) = first_of(doc) {
            let day = match first {
                First::Days(d, _) => d,
                First::Times(s, _) => match zone_of(doc) {
                    Some(tz) => s.with_timezone(&tz).date_naive(),
                    None => s.date_naive(),
                },
            };
            from = from.max(day);
        }
        let to = today + Duration::days(DAYS_AHEAD);
        let made = occurrences(doc, from, to).map_err(|e| {
            Error::Invalid(Invalid {
                file: note_path(EVENT_SERIES_TYPE, id),
                issues: vec![Issue::new("rule", e)],
            })
        })?;
        let before = old.filter(|o| is_local(o)).unwrap_or(doc);
        let mut have = HashSet::new();
        for (o, key, occ) in &mine {
            have.insert(key.clone());
            match made.iter().find(|m| &m.key == key) {
                Some(m) => {
                    let mut next = occ.clone();
                    for name in ["title", "location"] {
                        if text(occ, name) == text(before, name)
                            && text(occ, name) != text(doc, name)
                        {
                            set_text(&mut next, name, text(doc, name));
                        }
                    }
                    let description = held(doc, "description");
                    if held(occ, "description") == held(before, "description")
                        && held(occ, "description") != description
                    {
                        put(&mut next, "description", &description);
                    }
                    let span = when_of(occ);
                    if span == made_span(before, key) && span != span_of(&m.start, &m.end) {
                        next.fields.retain(|(n, _)| n != "when");
                        next.fields.push(("when".to_owned(), when(m)));
                    }
                    if next != *occ {
                        self.put_change(changes, o, EVENT_TYPE, Some(next), true)?;
                    }
                }
                None => {
                    if key_day(key).is_none_or(|d| d < from) {
                        continue;
                    }
                    let by_hand = !overrides(occ).is_empty()
                        || text(occ, "title") != text(before, "title")
                        || when_of(occ) != made_span(before, key);
                    if !by_hand {
                        self.put_change(changes, o, EVENT_TYPE, None, true)?;
                    }
                }
            }
        }
        for m in made.iter().filter(|m| !have.contains(&m.key)) {
            let occ = self.new_occurrence(id, doc, m);
            let o = ulid::Ulid::new().to_string();
            self.put_change(changes, &o, EVENT_TYPE, Some(occ), false)?;
        }
        Ok(())
    }

    /// A new occurrence `m` of series `id`.
    fn new_occurrence(&self, id: &str, series: &Doc, m: &Occurrence) -> Doc {
        let mut d = Doc::default();
        set_text(&mut d, "title", text(series, "title"));
        d.fields.push(("when".to_owned(), when(m)));
        for name in ["location", "status"] {
            set_text(&mut d, name, text(series, name));
        }
        put(&mut d, "description", &held(series, "description"));
        for f in &self.carried {
            put(&mut d, &f.name, &held(series, &f.name));
        }
        set_text(&mut d, "series_ref", id);
        set_text(&mut d, "series_key", &format!("{id}_{}", m.key));
        if let Some(ty) = self.inner.schema.type_def(EVENT_TYPE) {
            for f in &ty.fields {
                if let Some(v) = f.default.as_ref().filter(|_| d.field(&f.name).is_none()) {
                    d.fields.push((f.name.clone(), v.clone()));
                }
            }
        }
        d
    }
}

fn when(m: &Occurrence) -> Value {
    Value::Record(vec![
        ("start".to_owned(), Value::Str(m.start.clone())),
        ("end".to_owned(), Value::Str(m.end.clone())),
    ])
}

impl Writer {
    /// Make the new days' occurrences of every vault-only series, as the window moves on (E5):
    /// one write per series that gains or loses any, authored `task server`. The scheduler calls
    /// it once a day. Returns how many notes it changed.
    pub fn roll_series(&self) -> Result<usize> {
        if !self.is_home() {
            return Ok(0);
        }
        let ids: Vec<String> = {
            let inner = self.lock();
            let conn = inner.index.connection();
            let mut stmt = conn
                .prepare("SELECT id FROM notes WHERE type = ?1 ORDER BY id")
                .map_err(index::Error::from)?;
            let rows = stmt
                .query_map([EVENT_SERIES_TYPE], |r| r.get::<_, String>(0))
                .map_err(index::Error::from)?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row.map_err(index::Error::from)?);
            }
            ids
        };
        let author = Author::Task(crate::shipped::SERVER_AUTHOR.to_owned());
        let mut changed = 0;
        for id in ids {
            let _w = self.write_lock();
            let inner = self.lock();
            let mut pass = Pass::default();
            let carried = carried_fields(&inner.schema);
            let mut step = Step {
                writer: self,
                inner: &inner,
                carried,
                pass: &mut pass,
            };
            let Some(doc) = step.load(&id, EVENT_SERIES_TYPE)? else {
                continue;
            };
            if !is_local(&doc) {
                continue;
            }
            let mut changes = Vec::new();
            step.generate(&mut changes, &id, Some(&doc), &doc)?;
            if changes.is_empty() {
                continue;
            }
            changed += changes.len();
            self.apply(inner, changes, &|| Ok(()), &author)?;
        }
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn series(rule: &[&str], start: &str, end: &str, tz: &str) -> Doc {
        let mut d = Doc::default();
        d.fields.push((
            "rule".into(),
            Value::List(rule.iter().map(|l| Value::Str((*l).into())).collect()),
        ));
        d.fields.push((
            "starts".into(),
            Value::Record(vec![
                ("start".into(), Value::Str(start.into())),
                ("end".into(), Value::Str(end.into())),
            ]),
        ));
        if !tz.is_empty() {
            d.fields.push(("timezone".into(), Value::Str(tz.into())));
        }
        d
    }

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn starts(o: &[Occurrence]) -> Vec<&str> {
        o.iter().map(|o| o.start.as_str()).collect()
    }

    #[test]
    fn weekly_by_day_in_a_zone_keeps_the_local_time_across_a_clock_change() {
        // Melbourne leaves daylight saving on 5 April 2026.
        let s = series(
            &["RRULE:FREQ=WEEKLY;BYDAY=MO,WE;COUNT=6"],
            "2026-03-30T10:00:00+11:00",
            "2026-03-30T11:00:00+11:00",
            "Australia/Melbourne",
        );
        let o = occurrences(&s, day("2026-01-01"), day("2027-01-01")).unwrap();
        assert_eq!(
            starts(&o),
            [
                "2026-03-30T10:00:00+11:00",
                "2026-04-01T10:00:00+11:00",
                "2026-04-06T10:00:00+10:00",
                "2026-04-08T10:00:00+10:00",
                "2026-04-13T10:00:00+10:00",
                "2026-04-15T10:00:00+10:00",
            ]
        );
        assert_eq!(o[0].end, "2026-03-30T11:00:00+11:00");
        assert_eq!(o[0].key, "20260329T230000Z");
        assert_eq!(o[2].key, "20260406T000000Z");
    }

    #[test]
    fn monthly_by_month_day_and_by_set_pos() {
        let s = series(
            &["RRULE:FREQ=MONTHLY;BYMONTHDAY=31;COUNT=3"],
            "2026-01-31",
            "2026-01-31",
            "",
        );
        let o = occurrences(&s, day("2026-01-01"), day("2027-01-01")).unwrap();
        assert_eq!(starts(&o), ["2026-01-31", "2026-03-31", "2026-05-31"]);
        // The last weekday of each month.
        let s = series(
            &["RRULE:FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1;COUNT=2"],
            "2026-01-30T09:00:00Z",
            "2026-01-30T09:30:00Z",
            "",
        );
        let o = occurrences(&s, day("2026-01-01"), day("2027-01-01")).unwrap();
        assert_eq!(
            starts(&o),
            ["2026-01-30T09:00:00+00:00", "2026-02-27T09:00:00+00:00"]
        );
        // The second Tuesday.
        let s = series(
            &["RRULE:FREQ=MONTHLY;BYDAY=2TU;COUNT=2"],
            "2026-01-13",
            "2026-01-13",
            "",
        );
        let o = occurrences(&s, day("2026-01-01"), day("2027-01-01")).unwrap();
        assert_eq!(starts(&o), ["2026-01-13", "2026-02-10"]);
    }

    #[test]
    fn until_as_a_date_or_a_utc_time_and_exdates_in_each_form() {
        let s = series(
            &[
                "RRULE:FREQ=DAILY;UNTIL=20260105",
                "EXDATE;VALUE=DATE:20260102",
                "EXDATE:20260103T000000Z",
            ],
            "2026-01-01",
            "2026-01-02",
            "",
        );
        let o = occurrences(&s, day("2026-01-01"), day("2027-01-01")).unwrap();
        assert_eq!(starts(&o), ["2026-01-01", "2026-01-04", "2026-01-05"]);
        assert_eq!(o[0].end, "2026-01-02");
        assert_eq!(o[0].key, "20260101");

        let s = series(
            &[
                "RRULE:FREQ=DAILY;UNTIL=20260104T225959Z",
                "EXDATE;TZID=Australia/Melbourne:20260102T100000",
                "EXDATE:20260102T230000Z",
            ],
            "2026-01-01T10:00:00+11:00",
            "2026-01-01T10:30:00+11:00",
            "Australia/Melbourne",
        );
        let o = occurrences(&s, day("2026-01-01"), day("2027-01-01")).unwrap();
        assert_eq!(
            starts(&o),
            ["2026-01-01T10:00:00+11:00", "2026-01-04T10:00:00+11:00"]
        );
    }

    #[test]
    fn rdate_adds_and_the_window_bounds_what_comes_back() {
        let s = series(
            &["RRULE:FREQ=WEEKLY", "RDATE;VALUE=DATE:20260110"],
            "2026-01-01",
            "2026-01-01",
            "",
        );
        let o = occurrences(&s, day("2026-01-05"), day("2026-01-16")).unwrap();
        assert_eq!(starts(&o), ["2026-01-08", "2026-01-10", "2026-01-15"]);
    }

    #[test]
    fn rule_lines_are_checked() {
        let ok =
            |l: &[&str]| check_rule_lines(&l.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>());
        assert!(ok(&["RRULE:FREQ=WEEKLY;BYDAY=MO"]).is_ok());
        assert!(
            ok(&[
                "RRULE:FREQ=WEEKLY;UNTIL=20270630",
                "EXDATE;VALUE=DATE:20261109"
            ])
            .is_ok()
        );
        assert!(
            ok(&[
                "RRULE:FREQ=DAILY",
                "EXDATE;TZID=Europe/Berlin:20261109T100000"
            ])
            .is_ok()
        );
        assert!(ok(&["RDATE:20261109T100000Z"]).is_ok());
        let e = ok(&["RRULE:FREQ=FORTNIGHTLY"]).unwrap_err();
        assert!(
            e.starts_with("line 1, \"RRULE:FREQ=FORTNIGHTLY\", is not a recurrence rule"),
            "{e}"
        );
        let e = ok(&["RRULE:FREQ=DAILY", "every tuesday"]).unwrap_err();
        assert!(
            e.contains("line 2") && e.contains("is not an RRULE, EXDATE or RDATE line"),
            "{e}"
        );
        let e = ok(&["EXDATE:20261109T100000Z"]).unwrap_err();
        assert_eq!(e, "needs an RRULE or RDATE line");
        let e = ok(&[
            "RRULE:FREQ=DAILY",
            "EXDATE;TZID=Mars/Olympus:20261109T100000",
        ])
        .unwrap_err();
        assert!(e.contains("Mars/Olympus"), "{e}");
    }
}
