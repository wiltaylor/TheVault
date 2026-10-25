//! Planner notes: the built-in `dailynote`, `weeklynote`, `monthlynote`, `quarterlynote` and
//! `yearlynote` types, and the note for a day, an ISO week, a month, a quarter or a year.
//!
//! Every vault has the five types. Each can be customised but not deleted, and each always keeps
//! a required `date` field of kind `date`, the first day of its period, because that field is how
//! a period finds its note.

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

/// The name of the built-in weekly note type: one note for each ISO week.
pub const WEEKLY_TYPE: &str = "weeklynote";

/// The name of the built-in monthly note type.
pub const MONTHLY_TYPE: &str = "monthlynote";

/// The name of the built-in quarterly note type.
pub const QUARTERLY_TYPE: &str = "quarterlynote";

/// The name of the built-in yearly note type.
pub const YEARLY_TYPE: &str = "yearlynote";

/// The weekly note type a vault without it gets.
pub const DEFAULT_WEEKLY_TYPE: &str = include_str!("../defaults/weeklynote.wcl");

/// The monthly note type a vault without it gets.
pub const DEFAULT_MONTHLY_TYPE: &str = include_str!("../defaults/monthlynote.wcl");

/// The quarterly note type a vault without it gets.
pub const DEFAULT_QUARTERLY_TYPE: &str = include_str!("../defaults/quarterlynote.wcl");

/// The yearly note type a vault without it gets.
pub const DEFAULT_YEARLY_TYPE: &str = include_str!("../defaults/yearlynote.wcl");

/// A planner period: the span one planner note covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    /// One day.
    Day,
    /// One ISO week, Monday to Sunday.
    Week,
    /// One calendar month.
    Month,
    /// One quarter: January, April, July or October and the two months after it.
    Quarter,
    /// One calendar year.
    Year,
}

impl Period {
    /// Every period, shortest first.
    pub const ALL: [Period; 5] = [
        Period::Day,
        Period::Week,
        Period::Month,
        Period::Quarter,
        Period::Year,
    ];

    /// The period a route names: `day`, `week`, `month`, `quarter` or `year`.
    pub fn parse(name: &str) -> Option<Period> {
        Period::ALL.into_iter().find(|p| p.name() == name)
    }

    /// The period's name in a route, such as `week`.
    pub fn name(self) -> &'static str {
        match self {
            Period::Day => "day",
            Period::Week => "week",
            Period::Month => "month",
            Period::Quarter => "quarter",
            Period::Year => "year",
        }
    }

    /// The built-in type whose notes are this period's.
    pub fn type_name(self) -> &'static str {
        match self {
            Period::Day => DAILY_TYPE,
            Period::Week => WEEKLY_TYPE,
            Period::Month => MONTHLY_TYPE,
            Period::Quarter => QUARTERLY_TYPE,
            Period::Year => YEARLY_TYPE,
        }
    }

    /// The first day of the period holding `y`-`m`-`d`, which must be a real day.
    pub fn start(self, y: i32, m: u32, d: u32) -> (i32, u32, u32) {
        match self {
            Period::Day => (y, m, d),
            Period::Week => {
                let days = days_from_civil(y, m, d);
                civil_from_days(days - i64::from(weekday(days)))
            }
            Period::Month => (y, m, 1),
            Period::Quarter => (y, (m - 1) / 3 * 3 + 1, 1),
            Period::Year => (y, 1, 1),
        }
    }

    /// The title a new note of the period starting `y`-`m`-`d` gets: `Tuesday 13 October 2026`,
    /// `Week 43, 2026` (the ISO week and its week-year), `October 2026`, `Q4 2026` or `2026`.
    pub fn title(self, y: i32, m: u32, d: u32) -> String {
        match self {
            Period::Day => day_title(y, m, d),
            Period::Week => {
                let (year, week) = iso_week(y, m, d);
                format!("Week {week}, {year}")
            }
            Period::Month => format!("{} {y}", MONTHS[m as usize - 1]),
            Period::Quarter => format!("Q{} {y}", (m - 1) / 3 + 1),
            Period::Year => y.to_string(),
        }
    }
}

/// The ISO week of `y`-`m`-`d`: its week-year, the year of the week's Thursday, and its week
/// number, 1 to 53.
pub fn iso_week(y: i32, m: u32, d: u32) -> (i32, u32) {
    let days = days_from_civil(y, m, d);
    let thursday = days - i64::from(weekday(days)) + 3;
    let (ty, _, _) = civil_from_days(thursday);
    let first = days_from_civil(ty, 1, 1);
    (ty, ((thursday - first) / 7 + 1) as u32)
}

/// Days since 1970-01-01 (Howard Hinnant's days_from_civil).
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y2 = if m <= 2 { y - 1 } else { y } as i64;
    let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
    let yoe = y2 - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The day `days` after 1970-01-01 (Howard Hinnant's civil_from_days).
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = (yoe + era * 400 + i64::from(m <= 2)) as i32;
    (y, m, d)
}

/// The day of the week of `days` since 1970-01-01: 0 for Monday up to 6 for Sunday.
fn weekday(days: i64) -> u32 {
    // 1970-01-01 was a Thursday.
    (days + 3).rem_euclid(7) as u32
}

fn invalid(ty: &str, message: impl Into<String>) -> Error {
    Error::InvalidType(SchemaError(vec![Invalid {
        file: type_path(ty),
        issues: vec![Issue::new("type", message.into())],
    }]))
}

/// Refuse a schema whose planner types (`dailynote`, `weeklynote` and the rest) have lost their
/// `date` field, or changed it so it no longer holds one required date. A schema without one of
/// the types passes; [`Writer::ensure_built_in_types`] adds it.
pub(crate) fn check_daily_type(schema: &Schema) -> Result<()> {
    for period in Period::ALL {
        let name = period.type_name();
        let Some(ty) = schema.type_def(name) else {
            continue;
        };
        match ty.fields.iter().find(|f| f.name == DAILY_DATE_FIELD) {
            Some(f) if f.kind == FieldKind::Date && f.required && !f.multiple && !f.computed => {}
            Some(_) => {
                return Err(invalid(
                    name,
                    format!(
                        "the built-in {name} type's `{DAILY_DATE_FIELD}` field must stay one required date"
                    ),
                ));
            }
            None => {
                return Err(invalid(
                    name,
                    format!("the built-in {name} type must keep its `{DAILY_DATE_FIELD}` field"),
                ));
            }
        }
    }
    Ok(())
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

/// The first day of the `period` holding `date` (`YYYY-MM-DD`), as `YYYY-MM-DD`, or `None` when
/// `date` is not a day.
pub fn period_start(period: Period, date: &str) -> Option<String> {
    let (y, m, d) = parse_day(date)?;
    let (y, m, d) = period.start(y, m, d);
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

/// The title a new daily note gets, such as `Tuesday 13 October 2026`.
pub fn day_title(y: i32, m: u32, d: u32) -> String {
    let weekday = weekday(days_from_civil(y, m, d)) as usize;
    format!("{} {d} {} {y}", WEEKDAYS[weekday], MONTHS[m as usize - 1])
}

fn not_a_day(date: &str) -> Error {
    Error::BadRequest(format!("{date:?} is not a day: give it as YYYY-MM-DD"))
}

impl Writer {
    /// The daily note for `date` (`YYYY-MM-DD`), without creating one.
    pub fn find_daily_note(&self, date: &str) -> Result<Option<Note>> {
        self.find_period_note(Period::Day, date)
    }

    /// The daily note for `date` (`YYYY-MM-DD`), and whether this call created it: see
    /// [`Writer::period_note`].
    pub fn daily_note(&self, date: &str, author: &Author) -> Result<(Note, bool)> {
        self.period_note(Period::Day, date, author)
    }

    /// The note of the `period` holding `date` (`YYYY-MM-DD`, any day of the period), without
    /// creating one. An archived note still counts.
    pub fn find_period_note(&self, period: Period, date: &str) -> Result<Option<Note>> {
        let start = period_start(period, date).ok_or_else(|| not_a_day(date))?;
        let found = period_id(&self.lock(), period, &start)?;
        found.map(|id| self.read(&id)).transpose()
    }

    /// The note of the `period` holding `date` (`YYYY-MM-DD`, any day of the period), and
    /// whether this call created it. Finding and creating happen under the write lock, so two
    /// clients opening the same period get one note. When several notes share the period's first
    /// day, the oldest is the period's note; an archived one still counts.
    pub fn period_note(&self, period: Period, date: &str, author: &Author) -> Result<(Note, bool)> {
        let (y, m, d) = parse_day(date).ok_or_else(|| not_a_day(date))?;
        let (y, m, d) = period.start(y, m, d);
        let start = format!("{y:04}-{m:02}-{d:02}");
        let ty = period.type_name();
        let _w = self.write_lock();
        let inner = self.lock();
        if inner.schema.type_def(ty).is_none() {
            return Err(Error::UnknownType(ty.to_owned()));
        }
        let found = period_id(&inner, period, &start)?;
        if let Some(id) = found {
            drop(inner);
            return Ok((self.read(&id)?, false));
        }
        let id = ulid::Ulid::new().to_string();
        let wcl = format!(
            "title = {}\n{DAILY_DATE_FIELD} = {}\n",
            wcl_string(&period.title(y, m, d)),
            wcl_string(&start)
        );
        let doc = Input::Wcl(wcl).parse(&note_path(ty, &id))?;
        let change = self.prepare_put(&inner, &id, ty, doc, None, author)?;
        self.apply(inner, vec![change], &|| Ok(()), author)?;
        Ok((self.read(&id)?, true))
    }
}

/// The id of the oldest note of `period` whose date is `start`, if there is one.
fn period_id(inner: &crate::Inner, period: Period, start: &str) -> Result<Option<String>> {
    let conn = inner.index.connection();
    let mut stmt = conn
        .prepare(
            "SELECT n.id FROM notes n JOIN field_values f ON f.note = n.id
             WHERE n.type = ?1 AND f.field = ?2 AND f.str = ?3
             ORDER BY n.created_at, n.id LIMIT 1",
        )
        .map_err(index::Error::from)?;
    let mut rows = stmt
        .query_map([period.type_name(), DAILY_DATE_FIELD, start], |r| r.get(0))
        .map_err(index::Error::from)?;
    Ok(rows.next().transpose().map_err(index::Error::from)?)
}

fn wcl_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::{Period, day_title, iso_week, parse_day, period_start};

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

    #[test]
    fn iso_weeks_cross_year_ends() {
        // 2021-01-01 to 01-03 belong to week 53 of 2020; 2021-01-04 starts week 1.
        assert_eq!(iso_week(2021, 1, 1), (2020, 53));
        assert_eq!(iso_week(2021, 1, 3), (2020, 53));
        assert_eq!(iso_week(2021, 1, 4), (2021, 1));
        // 2023-01-01, a Sunday, is in week 52 of 2022.
        assert_eq!(iso_week(2023, 1, 1), (2022, 52));
        // 2024-12-30 and 2024-12-31 are in week 1 of 2025.
        assert_eq!(iso_week(2024, 12, 29), (2024, 52));
        assert_eq!(iso_week(2024, 12, 30), (2025, 1));
        assert_eq!(iso_week(2024, 12, 31), (2025, 1));
        // 2026 has 53 weeks: 2026-12-31 and 2027-01-01 to 01-03 are in week 53 of 2026.
        assert_eq!(iso_week(2026, 12, 31), (2026, 53));
        assert_eq!(iso_week(2027, 1, 3), (2026, 53));
        assert_eq!(iso_week(2027, 1, 4), (2027, 1));
        assert_eq!(iso_week(2026, 10, 25), (2026, 43));
    }

    #[test]
    fn dates_normalise_to_their_period_start() {
        let start = |p, d| period_start(p, d).unwrap();
        assert_eq!(start(Period::Day, "2026-10-25"), "2026-10-25");
        assert_eq!(start(Period::Week, "2026-10-25"), "2026-10-19");
        assert_eq!(start(Period::Week, "2026-10-19"), "2026-10-19");
        assert_eq!(start(Period::Week, "2021-01-01"), "2020-12-28");
        assert_eq!(start(Period::Week, "2024-12-31"), "2024-12-30");
        assert_eq!(start(Period::Week, "2025-01-01"), "2024-12-30");
        assert_eq!(start(Period::Week, "2024-03-01"), "2024-02-26");
        assert_eq!(start(Period::Month, "2026-02-28"), "2026-02-01");
        assert_eq!(start(Period::Quarter, "2026-12-31"), "2026-10-01");
        assert_eq!(start(Period::Quarter, "2026-03-31"), "2026-01-01");
        assert_eq!(start(Period::Quarter, "2026-05-15"), "2026-04-01");
        assert_eq!(start(Period::Quarter, "2026-07-01"), "2026-07-01");
        assert_eq!(start(Period::Year, "2026-10-25"), "2026-01-01");
        assert_eq!(period_start(Period::Week, "2026-02-30"), None);
    }

    #[test]
    fn period_titles() {
        assert_eq!(Period::Week.title(2026, 10, 19), "Week 43, 2026");
        // The week of 2024-12-30 is week 1 of 2025, and its title says so.
        assert_eq!(Period::Week.title(2024, 12, 30), "Week 1, 2025");
        assert_eq!(Period::Week.title(2020, 12, 28), "Week 53, 2020");
        assert_eq!(Period::Month.title(2026, 10, 1), "October 2026");
        assert_eq!(Period::Quarter.title(2026, 10, 1), "Q4 2026");
        assert_eq!(Period::Quarter.title(2026, 1, 1), "Q1 2026");
        assert_eq!(Period::Year.title(2026, 1, 1), "2026");
        assert_eq!(Period::Day.title(2026, 10, 13), "Tuesday 13 October 2026");
    }

    #[test]
    fn periods_parse_by_name() {
        for p in Period::ALL {
            assert_eq!(Period::parse(p.name()), Some(p));
        }
        assert_eq!(Period::parse("fortnight"), None);
        assert_eq!(Period::Week.type_name(), "weeklynote");
    }
}
