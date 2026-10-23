//! The string encodings of D23, as `docs/vault-format.md` "Value encodings" defines them. WCL
//! has no date, URL or reference type, so these are checked here.

/// The inline and accent palette (OQ-S4).
pub const PALETTE: [&str; 9] = [
    "gray", "brown", "orange", "yellow", "green", "blue", "purple", "pink", "red",
];

/// `[a-z][a-z0-9_]*`, at most 64 characters: type, taxonomy, task, script and field names.
pub fn is_name(s: &str) -> bool {
    let mut chars = s.chars();
    s.len() <= 64
        && matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// `[A-Za-z_][A-Za-z0-9_]*`: template, view, canvas item and dict key names.
pub fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A note or asset id: 26 upper-case Crockford base32 characters. The first is at most `7`, as
/// a ULID's 128 bits require.
pub fn is_ulid(s: &str) -> bool {
    const CROCKFORD: &str = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    s.len() == 26 && s.chars().all(|c| CROCKFORD.contains(c)) && s.as_bytes()[0] <= b'7'
}

pub fn is_palette(s: &str) -> bool {
    PALETTE.contains(&s)
}

/// The ink colour that stands for the theme's foreground: dark ink on light paper, light ink on
/// dark paper.
pub const DEFAULT_INK: &str = "ink";

/// `#rrggbb`, lower-case hex, or `ink`, the default ink.
pub fn is_ink_colour(s: &str) -> bool {
    let b = s.as_bytes();
    s == DEFAULT_INK
        || b.len() == 7
            && b[0] == b'#'
            && b[1..]
                .iter()
                .all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))
}

/// An ISO 8601 calendar date, `YYYY-MM-DD`, that exists.
pub fn is_date(s: &str) -> bool {
    parse_date(s).is_some()
}

/// An ISO 8601 date and time with seconds and an offset:
/// `YYYY-MM-DDTHH:MM:SS[.fff](Z|±HH:MM)`.
pub fn is_datetime(s: &str) -> bool {
    parse_datetime(s).is_some()
}

/// An ISO 8601 datetime in UTC, written with `Z`, as `created` and `updated` are.
pub fn is_utc_datetime(s: &str) -> bool {
    s.ends_with('Z') && is_datetime(s)
}

/// An ISO 8601 duration such as `P1Y2M`, `P3W`, `PT1M` or `PT1.5S`. At least one component;
/// only the seconds may carry a fraction.
pub fn is_duration(s: &str) -> bool {
    let Some(rest) = s.strip_prefix('P') else {
        return false;
    };
    let (date, time) = match rest.split_once('T') {
        Some((d, t)) => (d, Some(t)),
        None => (rest, None),
    };
    let mut any = false;
    if !components(date, &['Y', 'M', 'W', 'D'], false, &mut any) {
        return false;
    }
    if let Some(time) = time {
        let mut time_any = false;
        if !components(time, &['H', 'M', 'S'], true, &mut time_any) || !time_any {
            return false;
        }
        any = true;
    }
    any
}

/// Number-designator pairs in `units` order. `fraction` allows one on the final `S`.
fn components(mut s: &str, units: &[char], fraction: bool, any: &mut bool) -> bool {
    let mut next = 0;
    while !s.is_empty() {
        let end = s.find(|c: char| !c.is_ascii_digit() && c != '.');
        let Some(end) = end else { return false };
        let (num, rest) = s.split_at(end);
        let unit = rest.chars().next().unwrap_or(' ');
        let Some(pos) = units[next..].iter().position(|u| *u == unit) else {
            return false;
        };
        let ok_number = match num.split_once('.') {
            None => !num.is_empty(),
            Some((i, f)) => {
                fraction
                    && unit == 'S'
                    && !i.is_empty()
                    && !f.is_empty()
                    && f.chars().all(|c| c.is_ascii_digit())
            }
        };
        if !ok_number {
            return false;
        }
        next += pos + 1;
        *any = true;
        s = &rest[1..];
    }
    true
}

/// An absolute URL with a scheme. When the scheme is followed by `//`, the authority must not be
/// empty. No whitespace anywhere.
pub fn is_url(s: &str) -> bool {
    let Some((scheme, rest)) = s.split_once(':') else {
        return false;
    };
    let mut sc = scheme.chars();
    let scheme_ok = matches!(sc.next(), Some(c) if c.is_ascii_alphabetic())
        && sc.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if !scheme_ok || rest.is_empty() || s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return false;
    }
    match rest.strip_prefix("//") {
        Some(after) => {
            let authority = after.split(['/', '?', '#']).next().unwrap_or("");
            !authority.is_empty()
        }
        None => true,
    }
}

/// Days since 1970-01-01 of a valid `YYYY-MM-DD`.
pub(crate) fn parse_date(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let y = digits(&s[0..4])?;
    let m = digits(&s[5..7])?;
    let d = digits(&s[8..10])?;
    if !(1..=12).contains(&m) || d < 1 || d > days_in_month(y, m) {
        return None;
    }
    Some(days_from_civil(y, m, d))
}

/// Seconds since the Unix epoch, as a float to keep the fraction, of a valid datetime.
pub fn parse_datetime(s: &str) -> Option<f64> {
    if s.len() < 20 || s.as_bytes()[10] != b'T' {
        return None;
    }
    let days = parse_date(&s[..10])?;
    let t = &s[11..];
    let tb = t.as_bytes();
    if tb.len() < 9 || tb[2] != b':' || tb[5] != b':' {
        return None;
    }
    let (h, mi, sec) = (digits(&t[0..2])?, digits(&t[3..5])?, digits(&t[6..8])?);
    if h > 23 || mi > 59 || sec > 59 {
        return None;
    }
    let mut rest = &t[8..];
    let mut frac = 0.0;
    if let Some(after) = rest.strip_prefix('.') {
        let n = after
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(after.len());
        if n == 0 {
            return None;
        }
        frac = format!("0.{}", &after[..n]).parse().ok()?;
        rest = &after[n..];
    }
    let offset = if rest == "Z" {
        0
    } else {
        let sign = match rest.as_bytes().first()? {
            b'+' => 1,
            b'-' => -1,
            _ => return None,
        };
        let o = &rest[1..];
        if o.len() != 5 || o.as_bytes()[2] != b':' {
            return None;
        }
        let (oh, om) = (digits(&o[0..2])?, digits(&o[3..5])?);
        if oh > 23 || om > 59 {
            return None;
        }
        sign * (oh * 3600 + om * 60)
    };
    let secs = days * 86_400 + h * 3600 + mi * 60 + sec - offset;
    Some(secs as f64 + frac)
}

fn digits(s: &str) -> Option<i64> {
    if s.bytes().all(|c| c.is_ascii_digit()) {
        s.parse().ok()
    } else {
        None
    }
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Howard Hinnant's days-from-civil.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert!(is_date("2026-11-01"));
        assert!(is_date("2024-02-29"));
        assert!(!is_date("2026-02-29"));
        assert!(!is_date("2026-13-01"));
        assert!(!is_date("2026-1-01"));
        assert!(!is_date("2026-11-01T00:00:00Z"));
        assert_eq!(parse_date("1970-01-01"), Some(0));
    }

    #[test]
    fn datetimes() {
        assert!(is_datetime("2026-10-20T09:30:00+11:00"));
        assert!(is_datetime("2026-10-12T09:15:00Z"));
        assert!(is_datetime("2026-10-12T09:15:00.123Z"));
        assert!(!is_datetime("2026-10-12T09:15Z"));
        assert!(!is_datetime("2026-10-12T09:15:00"));
        assert!(!is_datetime("2026-10-12T24:00:00Z"));
        assert!(!is_datetime("2026-10-12 09:15:00Z"));
        assert_eq!(
            parse_datetime("2026-10-20T09:30:00+11:00"),
            parse_datetime("2026-10-19T22:30:00Z")
        );
        assert!(is_utc_datetime("2026-10-12T09:15:00Z"));
        assert!(!is_utc_datetime("2026-10-12T09:15:00+00:00"));
    }

    #[test]
    fn durations() {
        for ok in ["PT1M", "PT30S", "P1Y2M3DT4H5M6S", "P3W", "PT1.5S", "P1D"] {
            assert!(is_duration(ok), "{ok}");
        }
        for bad in ["P", "PT", "1M", "PT1.5M", "P1M1Y", "PT1", "P1DT"] {
            assert!(!is_duration(bad), "{bad}");
        }
    }

    #[test]
    fn urls() {
        assert!(is_url("https://example.com/importer"));
        assert!(is_url("mailto:wil@example.com"));
        assert!(!is_url("example.com"));
        assert!(!is_url("https://"));
        assert!(!is_url("https://exa mple.com"));
        assert!(!is_url("/relative/path"));
    }

    #[test]
    fn ulids_and_colours() {
        assert!(is_ulid("01JB8Z4K2MQ7V3X9T5R6W8Y2ZA"));
        assert!(!is_ulid("01jb8z4k2mq7v3x9t5r6w8y2za"));
        assert!(!is_ulid("01JB8Z4K2MQ7V3X9T5R6W8Y2Z"));
        assert!(!is_ulid("01JB8Z4K2MQ7V3X9T5R6W8Y2ZI"));
        assert!(!is_ulid("81JB8Z4K2MQ7V3X9T5R6W8Y2ZA"));
        assert!(is_ink_colour("#1f2937"));
        assert!(is_ink_colour("ink"));
        assert!(!is_ink_colour("Ink") && !is_ink_colour("black"));
        assert!(!is_ink_colour("#1F2937"));
        assert!(!is_ink_colour("1f2937"));
        assert!(is_palette("blue") && !is_palette("cyan"));
        assert!(is_name("in_progress2") && !is_name("InProgress") && !is_name("2x"));
    }
}
