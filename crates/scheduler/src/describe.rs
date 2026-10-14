//! A five-field cron expression in words, such as "Weekdays at 07:00", for the scheduled task
//! list and editor. Forms it does not know are described field by field.

const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
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

/// One cron field in a form this module can say.
#[derive(Debug, Clone, PartialEq)]
enum Field {
    /// `*`
    Any,
    /// `*/n`
    Every(u32),
    /// Single values and ranges, as written: `1`, `1-5`, `1,3,5`.
    Items(Vec<(u32, u32)>),
}

impl Field {
    fn single(&self) -> Option<u32> {
        match self {
            Field::Items(items) if items.len() == 1 && items[0].0 == items[0].1 => Some(items[0].0),
            _ => None,
        }
    }

    /// Every value, when the field is a list of single values.
    fn singles(&self) -> Option<Vec<u32>> {
        match self {
            Field::Items(items) if items.iter().all(|(a, b)| a == b) => {
                Some(items.iter().map(|(a, _)| *a).collect())
            }
            _ => None,
        }
    }
}

/// Parse one field. `names` maps three-letter names, such as `MON`, to their numbers.
fn field(s: &str, max: u32, names: &[&str], offset: u32) -> Option<Field> {
    if s == "*" || s == "?" {
        return Some(Field::Any);
    }
    if let Some(n) = s.strip_prefix("*/") {
        return n.parse().ok().filter(|n| *n > 0).map(Field::Every);
    }
    let value = |v: &str| -> Option<u32> {
        if let Ok(n) = v.parse::<u32>() {
            return (n <= max).then_some(n);
        }
        names
            .iter()
            .position(|name| name[..3].eq_ignore_ascii_case(v))
            .map(|i| i as u32 + offset)
    };
    let mut items = Vec::new();
    for part in s.split(',') {
        let item = match part.split_once('-') {
            Some((a, b)) => (value(a)?, value(b)?),
            None => {
                let v = value(part)?;
                (v, v)
            }
        };
        if item.0 > item.1 {
            return None;
        }
        items.push(item);
    }
    Some(Field::Items(items))
}

/// `a`, `a and b`, `a, b and c`.
fn and_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn time(h: u32, m: u32) -> String {
    format!("{h:02}:{m:02}")
}

fn day_name(d: u32) -> &'static str {
    DAYS[(d % 7) as usize]
}

/// The days of the week, or `None` for every day.
fn weekdays(f: &Field) -> Option<String> {
    let items = match f {
        Field::Any => return None,
        Field::Every(n) => return Some(format!("every {n} days of the week")),
        Field::Items(items) => items,
    };
    let mut days: Vec<u32> = items
        .iter()
        .flat_map(|(a, b)| *a..=*b)
        .map(|d| d % 7)
        .collect();
    days.sort_unstable();
    days.dedup();
    match days.as_slice() {
        [0, 1, 2, 3, 4, 5, 6] => None,
        [1, 2, 3, 4, 5] => Some("weekdays".into()),
        [0, 6] => Some("weekends".into()),
        _ => {
            if let [(a, b)] = items.as_slice()
                && a != b
            {
                return Some(format!("{} to {}", day_name(*a), day_name(*b)));
            }
            let names: Vec<String> = days.iter().map(|d| format!("{}s", day_name(*d))).collect();
            Some(and_list(&names))
        }
    }
}

fn ordinal(n: u32) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

/// The days of the month, or `None` for every day.
fn month_days(f: &Field) -> Option<String> {
    match f {
        Field::Any => None,
        Field::Every(n) => Some(format!("every {n} days of the month")),
        Field::Items(items) => {
            let parts: Vec<String> = items
                .iter()
                .map(|(a, b)| {
                    if a == b {
                        ordinal(*a)
                    } else {
                        format!("{} to {}", ordinal(*a), ordinal(*b))
                    }
                })
                .collect();
            Some(format!("the {} of the month", and_list(&parts)))
        }
    }
}

/// The months, or `None` for every month.
fn months(f: &Field) -> Option<String> {
    match f {
        Field::Any => None,
        Field::Every(n) => Some(format!("every {n} months")),
        Field::Items(items) => {
            let name = |m: u32| MONTHS[(m.clamp(1, 12) - 1) as usize].to_owned();
            let parts: Vec<String> = items
                .iter()
                .map(|(a, b)| {
                    if a == b {
                        name(*a)
                    } else {
                        format!("{} to {}", name(*a), name(*b))
                    }
                })
                .collect();
            Some(and_list(&parts))
        }
    }
}

/// When in the day the schedule fires: `At(..)` for set times, `Often(..)` for a frequency.
enum Times {
    At(String),
    Often(String),
}

fn times(minute: &Field, hour: &Field) -> Option<Times> {
    use Field::{Any, Every};
    Some(match (minute, hour) {
        (Any, Any) => Times::Often("every minute".into()),
        (Every(n), Any) => Times::Often(format!("every {n} minutes")),
        (Any, h) => {
            let h = h.single()?;
            Times::Often(format!(
                "every minute from {} to {}",
                time(h, 0),
                time(h, 59)
            ))
        }
        (Every(n), Field::Items(items)) if items.len() == 1 => {
            let (a, b) = items[0];
            Times::Often(format!(
                "every {n} minutes from {} to {}",
                time(a, 0),
                time(b, 59)
            ))
        }
        (m, Any) => {
            let m = m.singles()?;
            if m == [0] {
                Times::Often("every hour".into())
            } else {
                let past: Vec<String> = m.iter().map(|m| format!(":{m:02}")).collect();
                Times::Often(format!("every hour at {}", and_list(&past)))
            }
        }
        (m, Every(n)) => {
            let m = m.single()?;
            let base = if *n == 1 {
                "every hour".to_owned()
            } else {
                format!("every {n} hours")
            };
            if m == 0 {
                Times::Often(base)
            } else {
                Times::Often(format!("{base} at :{m:02}"))
            }
        }
        (m, Field::Items(items)) => {
            let m = m.singles()?;
            if let [(a, b)] = items.as_slice()
                && a != b
            {
                let past: Vec<String> = m.iter().map(|m| format!(":{m:02}")).collect();
                return Some(Times::Often(format!(
                    "every hour at {} from {} to {}",
                    and_list(&past),
                    time(*a, m[0]),
                    time(*b, *m.last().unwrap_or(&0))
                )));
            }
            let hours = hour.singles()?;
            let mut all: Vec<String> = Vec::new();
            for h in &hours {
                for m in &m {
                    all.push(time(*h, *m));
                }
            }
            if all.len() > 8 {
                return None;
            }
            Times::At(and_list(&all))
        }
    })
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// `schedule`, a five-field cron expression the scheduler accepts, in words.
pub fn describe(schedule: &str) -> String {
    describe_fields(schedule).unwrap_or_else(|| format!("Custom schedule ({schedule})"))
}

fn describe_fields(schedule: &str) -> Option<String> {
    let f: Vec<&str> = schedule.split_whitespace().collect();
    let [minute, hour, dom, month, dow] = f.as_slice() else {
        return None;
    };
    let minute = field(minute, 59, &[], 0)?;
    let hour = field(hour, 23, &[], 0)?;
    let dom = field(dom, 31, &[], 0)?;
    let month = field(month, 12, &MONTHS, 1)?;
    let dow = field(dow, 7, &DAYS, 0)?;

    let times = times(&minute, &hour)?;
    let week = weekdays(&dow);
    let mdays = month_days(&dom);
    let months = months(&month);

    let days = match (&week, &mdays) {
        (Some(w), None) => Some(w.clone()),
        (None, Some(d)) => Some(format!("on {d}")),
        // Cron fires when either matches.
        (Some(w), Some(d)) => Some(format!("on {d} and on {w}")),
        (None, None) => None,
    };
    let mut out = match (times, &days) {
        (Times::At(t), Some(d)) => format!("{} at {t}", capitalise(d)),
        (Times::At(t), None) => format!("Every day at {t}"),
        (Times::Often(t), Some(d)) if week.is_some() && mdays.is_none() => {
            format!("{} on {d}", capitalise(&t))
        }
        (Times::Often(t), Some(d)) => format!("{} {d}", capitalise(&t)),
        (Times::Often(t), None) => capitalise(&t),
    };
    if let Some(m) = months {
        out.push_str(&format!(", in {m}"));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::describe;

    #[test]
    fn common_schedules_read_as_words() {
        for (cron, words) in [
            ("0 7 * * 1-5", "Weekdays at 07:00"),
            ("30 6 * * MON-FRI", "Weekdays at 06:30"),
            ("0 10 * * 0,6", "Weekends at 10:00"),
            ("0 9 * * 1", "Mondays at 09:00"),
            ("0 9 * * 1,3,5", "Mondays, Wednesdays and Fridays at 09:00"),
            ("0 9 * * 1-4", "Monday to Thursday at 09:00"),
            ("15 8 * * *", "Every day at 08:15"),
            ("0 7,19 * * *", "Every day at 07:00 and 19:00"),
            ("0 * * * *", "Every hour"),
            ("30 * * * *", "Every hour at :30"),
            ("* * * * *", "Every minute"),
            ("*/15 * * * *", "Every 15 minutes"),
            ("0 */6 * * *", "Every 6 hours"),
            ("0 0 1 * *", "On the 1st of the month at 00:00"),
            ("0 0 1,15 * *", "On the 1st and 15th of the month at 00:00"),
            ("0 0 1 1 *", "On the 1st of the month at 00:00, in January"),
            (
                "0 9-17 * * 1-5",
                "Every hour at :00 from 09:00 to 17:00 on weekdays",
            ),
            ("0 0 * * 7", "Sundays at 00:00"),
        ] {
            assert_eq!(describe(cron), words, "{cron}");
        }
    }

    #[test]
    fn unknown_forms_fall_back_to_the_expression() {
        assert_eq!(describe("0 0 L * *"), "Custom schedule (0 0 L * *)");
    }
}
