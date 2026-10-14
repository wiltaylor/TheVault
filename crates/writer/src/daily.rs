//! Daily notes: the built-in `dailynote` type and the note for a day.
//!
//! Every vault has the `dailynote` type. It can be customised but not deleted, and it always keeps
//! a required `date` field of kind `date`, because that field is how a day finds its note.

use schema::{FieldKind, Invalid, Issue, Schema, SchemaError};
use state::Author;

use crate::{Error, Input, Note, Result, Writer, note_path, type_path};

/// The name of the built-in daily note type.
pub const DAILY_TYPE: &str = "dailynote";

/// The field that holds a daily note's day, as `YYYY-MM-DD`.
pub const DAILY_DATE_FIELD: &str = "date";

/// The type a vault gets when it has none, and the one a missing type file is recreated from.
pub const DEFAULT_DAILY_TYPE: &str = include_str!("../defaults/dailynote.wcl");

const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidType(SchemaError(vec![Invalid {
        file: type_path(DAILY_TYPE),
        issues: vec![Issue::new("type", message.into())],
    }]))
}

/// Refuse a schema whose `dailynote` type has lost its `date` field, or changed it so it no longer
/// holds one required date. A schema without the type passes; [`Writer::ensure_built_in_types`] adds it.
pub(crate) fn check_daily_type(schema: &Schema) -> Result<()> {
    let Some(ty) = schema.type_def(DAILY_TYPE) else {
        return Ok(());
    };
    match ty.fields.iter().find(|f| f.name == DAILY_DATE_FIELD) {
        Some(f) if f.kind == FieldKind::Date && f.required && !f.multiple && !f.computed => Ok(()),
        Some(_) => Err(invalid(format!(
            "the built-in {DAILY_TYPE} type's `{DAILY_DATE_FIELD}` field must stay one required date"
        ))),
        None => Err(invalid(format!(
            "the built-in {DAILY_TYPE} type must keep its `{DAILY_DATE_FIELD}` field"
        ))),
    }
}

/// `date` as (year, month, day) when it is a real `YYYY-MM-DD` day.
pub fn parse_day(date: &str) -> Option<(i32, u32, u32)> {
    let b = date.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let num = |s: &str| {
        s.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| s.parse::<u32>().ok())
            .flatten()
    };
    let y = num(&date[0..4])? as i32;
    let m: u32 = num(&date[5..7])?;
    let d: u32 = num(&date[8..10])?;
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    (1..=days).contains(&d).then_some((y, m, d))
}

/// The title a new daily note gets, such as `Tuesday 13 October 2026`.
pub fn day_title(y: i32, m: u32, d: u32) -> String {
    // Days since 1970-01-01, a Thursday (Howard Hinnant's days_from_civil).
    let y2 = if m <= 2 { y - 1 } else { y } as i64;
    let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
    let yoe = y2 - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let weekday = (days + 3).rem_euclid(7) as usize;
    format!("{} {d} {} {y}", WEEKDAYS[weekday], MONTHS[m as usize - 1])
}

impl Writer {
    /// The daily note for `date` (`YYYY-MM-DD`), without creating one.
    pub fn find_daily_note(&self, date: &str) -> Result<Option<Note>> {
        if parse_day(date).is_none() {
            return Err(Error::BadRequest(format!(
                "{date:?} is not a day: give it as YYYY-MM-DD"
            )));
        }
        let found = daily_id(&self.lock(), date)?;
        found.map(|id| self.read(&id)).transpose()
    }

    /// The daily note for `date` (`YYYY-MM-DD`), and whether this call created it. Finding and
    /// creating happen under the write lock, so two clients opening the same day get one note.
    /// When several notes share the day, the oldest is the day's note.
    pub fn daily_note(&self, date: &str, author: &Author) -> Result<(Note, bool)> {
        let Some((y, m, d)) = parse_day(date) else {
            return Err(Error::BadRequest(format!(
                "{date:?} is not a day: give it as YYYY-MM-DD"
            )));
        };
        let _w = self.write_lock();
        let inner = self.lock();
        if inner.schema.type_def(DAILY_TYPE).is_none() {
            return Err(Error::UnknownType(DAILY_TYPE.to_owned()));
        }
        let found = daily_id(&inner, date)?;
        if let Some(id) = found {
            drop(inner);
            return Ok((self.read(&id)?, false));
        }
        let id = ulid::Ulid::new().to_string();
        let wcl = format!(
            "title = {}\n{DAILY_DATE_FIELD} = {}\n",
            wcl_string(&day_title(y, m, d)),
            wcl_string(date)
        );
        let doc = Input::Wcl(wcl).parse(&note_path(DAILY_TYPE, &id))?;
        let change = self.prepare_put(&inner, &id, DAILY_TYPE, doc, None, author)?;
        self.apply(inner, vec![change], &|| Ok(()), author)?;
        Ok((self.read(&id)?, true))
    }
}

/// The id of the oldest note for `date`, if there is one.
fn daily_id(inner: &crate::Inner, date: &str) -> Result<Option<String>> {
    let conn = inner.index.connection();
    let mut stmt = conn
        .prepare(
            "SELECT n.id FROM notes n JOIN field_values f ON f.note = n.id
             WHERE n.type = ?1 AND f.field = ?2 AND f.str = ?3
             ORDER BY n.created_at, n.id LIMIT 1",
        )
        .map_err(index::Error::from)?;
    let mut rows = stmt
        .query_map([DAILY_TYPE, DAILY_DATE_FIELD, date], |r| r.get(0))
        .map_err(index::Error::from)?;
    Ok(rows.next().transpose().map_err(index::Error::from)?)
}

fn wcl_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::{day_title, parse_day};

    #[test]
    fn days_parse_only_when_they_exist() {
        assert_eq!(parse_day("2026-10-13"), Some((2026, 10, 13)));
        assert_eq!(parse_day("2024-02-29"), Some((2024, 2, 29)));
        for bad in [
            "2026-02-29",
            "2026-13-01",
            "2026-00-10",
            "2026-1-01",
            "26-10-13",
            "2026/10/13",
            "2026-10-1x",
        ] {
            assert_eq!(parse_day(bad), None, "{bad}");
        }
    }

    #[test]
    fn titles_name_the_weekday() {
        assert_eq!(day_title(2026, 10, 13), "Tuesday 13 October 2026");
        assert_eq!(day_title(2000, 1, 1), "Saturday 1 January 2000");
        assert_eq!(day_title(2024, 2, 29), "Thursday 29 February 2024");
        assert_eq!(day_title(1970, 1, 1), "Thursday 1 January 1970");
    }
}
