//! `tasks/<task>.wcl`, as `docs/vault-format.md` "Task files" defines it (D10).

use std::str::FromStr;
use std::time::Duration;

use chrono::TimeZone;
use chrono_tz::Tz;
use croner::Cron;
use schema::{Doc, Invalid, Issue, Value};

/// The fields a task file may hold.
const FIELDS: [&str; 12] = [
    "label",
    "description",
    "schedule",
    "timezone",
    "script",
    "function",
    "allow_domains",
    "secrets",
    "retries",
    "retry_delay",
    "timeout",
    "paused",
];

/// The most retries a task may ask for. The delay doubles each time, so more is never useful.
pub const MAX_RETRIES: i64 = 20;

/// One scheduled task.
#[derive(Debug, Clone)]
pub struct TaskDef {
    pub name: String,
    pub label: String,
    pub description: Option<String>,
    /// The five-field cron expression as written.
    pub schedule: String,
    cron: Cron,
    pub timezone: Tz,
    pub script: String,
    pub function: String,
    pub allow_domains: Vec<String>,
    pub secrets: Vec<String>,
    pub retries: u32,
    pub retry_delay: Duration,
    pub timeout: Duration,
    pub paused: bool,
}

impl PartialEq for TaskDef {
    fn eq(&self, other: &TaskDef) -> bool {
        // `schedule` is what `cron` was parsed from.
        self.name == other.name
            && self.label == other.label
            && self.description == other.description
            && self.schedule == other.schedule
            && self.timezone == other.timezone
            && self.script == other.script
            && self.function == other.function
            && self.allow_domains == other.allow_domains
            && self.secrets == other.secrets
            && self.retries == other.retries
            && self.retry_delay == other.retry_delay
            && self.timeout == other.timeout
            && self.paused == other.paused
    }
}

impl TaskDef {
    /// Read task `name` from the text of `tasks/<name>.wcl`. Every problem is reported at once.
    pub fn parse(name: &str, source: &str) -> Result<TaskDef, Invalid> {
        let file = format!("tasks/{name}.wcl");
        let doc = Doc::parse(source, &file)?;
        let mut issues = Vec::new();
        for b in &doc.blocks {
            issues.push(Issue::new(
                b.name(),
                "a task file holds fields only, not blocks",
            ));
        }
        for (field, _) in &doc.fields {
            if !FIELDS.contains(&field.as_str()) {
                issues.push(Issue::new(field, "is not a task field"));
            }
        }
        let mut r = Reader {
            doc: &doc,
            issues: &mut issues,
        };

        let label = r.string("label").unwrap_or_else(|| name.to_owned());
        let description = r.string("description");
        let schedule = r.required("schedule");
        let cron = schedule.as_deref().and_then(|s| match parse_cron(s) {
            Ok(c) => Some(c),
            Err(e) => {
                r.issue("schedule", e);
                None
            }
        });
        let timezone = match r.string("timezone") {
            None => Some(Tz::UTC),
            Some(z) => match Tz::from_str(&z) {
                Ok(tz) => Some(tz),
                Err(_) => {
                    r.issue("timezone", format!("{z:?} is not an IANA time zone"));
                    None
                }
            },
        };
        let script = r.required("script");
        if let Some(s) = &script
            && !schema::is_name(s)
        {
            r.issue("script", format!("{s:?} is not a script name"));
        }
        let function = r.string("function").unwrap_or_else(|| "run".to_owned());
        if !schema::is_ident(&function) {
            r.issue("function", format!("{function:?} is not a function name"));
        }
        let allow_domains = r.strings("allow_domains");
        for d in &allow_domains {
            if !is_host(d) {
                r.issue("allow_domains", format!("{d:?} is not a host name"));
            }
        }
        let secrets = r.strings("secrets");
        for s in &secrets {
            if !schema::is_name(s) {
                r.issue("secrets", format!("{s:?} is not a secret name"));
            }
        }
        let retries = match doc.field("retries") {
            None => 3,
            Some(Value::Int(n)) if (0..=MAX_RETRIES).contains(n) => *n as u32,
            Some(_) => {
                r.issue(
                    "retries",
                    format!("must be an integer from 0 to {MAX_RETRIES}"),
                );
                0
            }
        };
        let retry_delay = r.duration("retry_delay", Duration::from_secs(60));
        let timeout = r.duration("timeout", Duration::from_secs(30));
        let paused = match doc.field("paused") {
            None => false,
            Some(Value::Bool(b)) => *b,
            Some(v) => {
                r.issue(
                    "paused",
                    format!("must be true or false, not {}", v.describe()),
                );
                false
            }
        };

        match (schedule, cron, timezone, script) {
            (Some(schedule), Some(cron), Some(timezone), Some(script)) if issues.is_empty() => {
                Ok(TaskDef {
                    name: name.to_owned(),
                    label,
                    description,
                    schedule,
                    cron,
                    timezone,
                    script,
                    function,
                    allow_domains,
                    secrets,
                    retries,
                    retry_delay,
                    timeout,
                    paused,
                })
            }
            _ => Err(Invalid { file, issues }),
        }
    }

    /// The first time the schedule fires strictly after `after_ms`, in milliseconds since the
    /// Unix epoch, read in the task's time zone. `None` when it never fires again.
    pub fn next_after(&self, after_ms: i64) -> Option<i64> {
        let at = self.timezone.timestamp_millis_opt(after_ms).single()?;
        self.cron
            .find_next_occurrence(&at, false)
            .ok()
            .map(|t| t.timestamp_millis())
    }

    /// The delay before retry `retry`, counting from 1: `retry_delay`, doubling each time.
    pub fn backoff(&self, retry: u32) -> Duration {
        self.retry_delay
            .saturating_mul(2u32.saturating_pow(retry.saturating_sub(1)))
    }
}

fn parse_cron(s: &str) -> Result<Cron, String> {
    if s.split_whitespace().count() != 5 {
        return Err(format!(
            "{s:?} is not a five-field cron expression: minute, hour, day of month, month, day of week"
        ));
    }
    Cron::new(s)
        .parse()
        .map_err(|e| format!("{s:?} is not a valid cron expression: {e}"))
}

/// A DNS name or an IP address, without a scheme, port or path.
fn is_host(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 253
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'))
        && !s.starts_with(['.', '-'])
}

/// An ISO 8601 duration of weeks, days, hours, minutes and seconds. Years and months have no
/// fixed length, so a delay cannot use them.
pub fn parse_duration(s: &str) -> Result<Duration, String> {
    let bad = || format!("{s:?} is not an ISO 8601 duration such as \"PT1M\"");
    if !schema::is_duration(s) {
        return Err(bad());
    }
    let rest = &s[1..];
    let (date, time) = rest.split_once('T').unwrap_or((rest, ""));
    let mut secs = 0f64;
    for (part, units) in [
        (
            date,
            &[('Y', 0.0), ('M', 0.0), ('W', 604_800.0), ('D', 86_400.0)][..],
        ),
        (time, &[('H', 3_600.0), ('M', 60.0), ('S', 1.0)][..]),
    ] {
        let mut num = String::new();
        for c in part.chars() {
            if c.is_ascii_digit() || c == '.' {
                num.push(c);
                continue;
            }
            let scale = units.iter().find(|(u, _)| *u == c).map(|(_, f)| *f);
            let n: f64 = num.parse().map_err(|_| bad())?;
            match scale {
                Some(f) if f > 0.0 => secs += n * f,
                _ => {
                    return Err(format!(
                        "{s:?} uses years or months, which have no fixed length"
                    ));
                }
            }
            num.clear();
        }
    }
    Ok(Duration::from_secs_f64(secs))
}

struct Reader<'a> {
    doc: &'a Doc,
    issues: &'a mut Vec<Issue>,
}

impl Reader<'_> {
    fn issue(&mut self, at: &str, message: impl Into<String>) {
        self.issues.push(Issue::new(at, message));
    }

    fn string(&mut self, field: &str) -> Option<String> {
        match self.doc.field(field)? {
            Value::Str(s) => Some(s.clone()),
            v => {
                let msg = format!("must be a string, not {}", v.describe());
                self.issue(field, msg);
                None
            }
        }
    }

    fn required(&mut self, field: &str) -> Option<String> {
        if self.doc.field(field).is_none() {
            self.issue(field, "is required");
        }
        self.string(field)
    }

    fn strings(&mut self, field: &str) -> Vec<String> {
        match self.doc.field(field) {
            None => Vec::new(),
            Some(Value::List(items)) => {
                let mut out = Vec::new();
                for v in items {
                    match v {
                        Value::Str(s) => out.push(s.clone()),
                        v => {
                            let msg = format!("must hold strings only, not {}", v.describe());
                            self.issue(field, msg);
                        }
                    }
                }
                out
            }
            Some(v) => {
                let msg = format!("must be a list of strings, not {}", v.describe());
                self.issue(field, msg);
                Vec::new()
            }
        }
    }

    fn duration(&mut self, field: &str, default: Duration) -> Duration {
        let Some(s) = self.string(field) else {
            return default;
        };
        match parse_duration(&s) {
            Ok(d) if !d.is_zero() => d,
            Ok(_) => {
                self.issue(field, "must be longer than zero");
                default
            }
            Err(e) => {
                self.issue(field, e);
                default
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = r#"label         = "Daily digest"
schedule      = "0 7 * * 1-5"
timezone      = "Australia/Melbourne"
script        = "digest"
allow_domains = ["hooks.example.com"]
secrets       = ["digest_webhook"]
retries       = 3
retry_delay   = "PT1M"
"#;

    #[test]
    fn the_format_example_parses_with_defaults() {
        let t = TaskDef::parse("daily_digest", DIGEST).unwrap();
        assert_eq!(t.label, "Daily digest");
        assert_eq!(t.function, "run");
        assert_eq!(t.timeout, Duration::from_secs(30));
        assert_eq!(t.retry_delay, Duration::from_secs(60));
        assert_eq!(t.timezone, chrono_tz::Australia::Melbourne);
        assert!(!t.paused);
    }

    #[test]
    fn next_after_reads_the_schedule_in_the_task_time_zone() {
        let t = TaskDef::parse("daily_digest", DIGEST).unwrap();
        // Monday 2026-10-12 00:00 UTC is 11:00 in Melbourne (AEDT, +11), past 07:00, so the next
        // run is Tuesday 07:00 Melbourne: Monday 20:00 UTC.
        let monday = chrono::Utc
            .with_ymd_and_hms(2026, 10, 12, 0, 0, 0)
            .unwrap()
            .timestamp_millis();
        let next = chrono::Utc
            .with_ymd_and_hms(2026, 10, 12, 20, 0, 0)
            .unwrap()
            .timestamp_millis();
        assert_eq!(t.next_after(monday), Some(next));
        assert_eq!(t.next_after(next), Some(next + 24 * 3_600_000));
    }

    #[test]
    fn backoff_doubles() {
        let t = TaskDef::parse("daily_digest", DIGEST).unwrap();
        let mins: Vec<u64> = (1..=3).map(|n| t.backoff(n).as_secs() / 60).collect();
        assert_eq!(mins, [1, 2, 4]);
    }

    #[test]
    fn every_problem_is_reported() {
        let src = r#"schedule = "every day"
timezone = "Mars/Olympus"
function = "not a name"
retries = -1
retry_delay = "P1M"
colour = "red"
"#;
        let err = TaskDef::parse("t", src).unwrap_err();
        let at: Vec<&str> = err.issues.iter().map(|i| i.at.as_str()).collect();
        assert_eq!(
            at,
            [
                "colour",
                "schedule",
                "timezone",
                "script",
                "function",
                "retries",
                "retry_delay"
            ],
            "{err:?}"
        );
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("PT1M"), Ok(Duration::from_secs(60)));
        assert_eq!(parse_duration("PT1.5S"), Ok(Duration::from_millis(1500)));
        assert_eq!(parse_duration("P1DT1H"), Ok(Duration::from_secs(90_000)));
        assert!(parse_duration("P1Y").is_err());
        assert!(parse_duration("1m").is_err());
    }
}
