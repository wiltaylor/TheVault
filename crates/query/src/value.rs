//! The values an expression works with, and how stored field values become them.

use std::cmp::Ordering;

use schema::{FieldKind, RangeOf};

/// A value during evaluation. Dates and datetimes are typed by the schema, so they compare in
/// time order rather than as text.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum V {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    /// Days since 1970-01-01.
    Date(i64),
    /// Seconds since the Unix epoch.
    Datetime(f64),
    /// Seconds.
    Duration(f64),
    /// The id of a note.
    Ref(String),
    List(Vec<V>),
    Record(Vec<(String, V)>),
}

pub(crate) type Eval<T> = Result<T, String>;

impl V {
    pub fn kind(&self) -> &'static str {
        match self {
            V::Null => "nothing",
            V::Bool(_) => "a bool",
            V::Int(_) => "an int",
            V::Float(_) => "a float",
            V::Str(_) => "a string",
            V::Date(_) => "a date",
            V::Datetime(_) => "a datetime",
            V::Duration(_) => "a duration",
            V::Ref(_) => "a note",
            V::List(_) => "a list",
            V::Record(_) => "a record",
        }
    }

    /// Truth in `&&`, `||`, `!` and a `where` clause. A missing value is false.
    pub fn truth(&self) -> Eval<bool> {
        match self {
            V::Bool(b) => Ok(*b),
            V::Null => Ok(false),
            v => Err(format!("expected a bool, found {}", v.kind())),
        }
    }

    fn time(&self) -> Option<f64> {
        match self {
            V::Date(d) => Some(*d as f64 * 86_400.0),
            V::Datetime(t) => Some(*t),
            _ => None,
        }
    }

    fn num(&self) -> Option<f64> {
        match self {
            V::Int(n) => Some(*n as f64),
            V::Float(x) => Some(*x),
            _ => None,
        }
    }

    fn id(&self) -> Option<&str> {
        match self {
            V::Str(s) | V::Ref(s) => Some(s),
            _ => None,
        }
    }
}

/// A string read as a date or datetime, so `due < "2026-11-01"` compares dates.
pub(crate) fn str_time(s: &str) -> Option<f64> {
    time::date(s)
        .map(|d| d as f64 * 86_400.0)
        .or_else(|| time::datetime(s))
}

/// The two sides of a comparison as times, when one is a time and the other a time or a string
/// holding one.
fn times(a: &V, b: &V) -> Option<(f64, f64)> {
    match (a.time(), b.time()) {
        (Some(x), Some(y)) => Some((x, y)),
        (Some(x), None) => Some((x, str_time(b.id()?)?)),
        (None, Some(y)) => Some((str_time(a.id()?)?, y)),
        (None, None) => None,
    }
}

pub(crate) fn equals(a: &V, b: &V) -> Eval<bool> {
    if let Some((x, y)) = times(a, b) {
        return Ok(x == y);
    }
    Ok(match (a, b) {
        (V::Null, V::Null) => true,
        (V::Null, _) | (_, V::Null) => false,
        (V::Bool(x), V::Bool(y)) => x == y,
        (V::Duration(x), V::Duration(y)) => x == y,
        (V::List(x), V::List(y)) => {
            x.len() == y.len()
                && x.iter()
                    .zip(y)
                    .try_fold(true, |acc, (p, q)| Ok::<_, String>(acc && equals(p, q)?))?
        }
        (V::Record(x), V::Record(y)) => {
            x.len() == y.len()
                && x.iter().zip(y).try_fold(true, |acc, ((k, p), (l, q))| {
                    Ok::<_, String>(acc && k == l && equals(p, q)?)
                })?
        }
        _ => match (a.num(), b.num(), a.id(), b.id()) {
            (Some(x), Some(y), ..) => x == y,
            (.., Some(x), Some(y)) => x == y,
            _ => return Err(format!("cannot compare {} with {}", a.kind(), b.kind())),
        },
    })
}

/// `None` when either side is missing, which makes every ordering comparison false.
pub(crate) fn compare(a: &V, b: &V) -> Eval<Option<Ordering>> {
    if matches!(a, V::Null) || matches!(b, V::Null) {
        return Ok(None);
    }
    if let Some((x, y)) = times(a, b) {
        return Ok(x.partial_cmp(&y));
    }
    match (a, b) {
        (V::Duration(x), V::Duration(y)) => Ok(x.partial_cmp(y)),
        (V::Str(x), V::Str(y)) => Ok(Some(x.cmp(y))),
        _ => match (a.num(), b.num()) {
            (Some(x), Some(y)) => Ok(x.partial_cmp(&y)),
            _ => Err(format!("cannot order {} against {}", a.kind(), b.kind())),
        },
    }
}

/// `a + b` when `add`, else `a - b`.
pub(crate) fn add_sub(a: V, b: V, add: bool) -> Eval<V> {
    let sign = if add { 1.0 } else { -1.0 };
    let op = if add { "+" } else { "-" };
    Ok(match (a, b) {
        (V::Null, _) | (_, V::Null) => V::Null,
        (V::Int(x), V::Int(y)) => {
            let r = if add {
                x.checked_add(y)
            } else {
                x.checked_sub(y)
            };
            V::Int(r.ok_or("integer overflow")?)
        }
        (V::Date(d), V::Duration(s)) => {
            let secs = d as f64 * 86_400.0 + sign * s;
            if s % 86_400.0 == 0.0 {
                V::Date((secs / 86_400.0) as i64)
            } else {
                V::Datetime(secs)
            }
        }
        (V::Datetime(t), V::Duration(s)) => V::Datetime(t + sign * s),
        (V::Duration(x), V::Duration(y)) => V::Duration(x + sign * y),
        (V::Duration(s), V::Date(d)) if add => add_sub(V::Date(d), V::Duration(s), true)?,
        (V::Duration(s), V::Datetime(t)) if add => V::Datetime(t + s),
        (a @ (V::Date(_) | V::Datetime(_)), b @ (V::Date(_) | V::Datetime(_))) if !add => {
            V::Duration(a.time().unwrap_or_default() - b.time().unwrap_or_default())
        }
        (V::Str(x), V::Str(y)) if add => V::Str(x + &y),
        (a, b) => match (a.num(), b.num()) {
            (Some(x), Some(y)) => V::Float(x + sign * y),
            _ => {
                return Err(format!(
                    "cannot apply `{op}` to {} and {}",
                    a.kind(),
                    b.kind()
                ));
            }
        },
    })
}

/// `*`, `/` and `%`.
pub(crate) fn mul_div(a: V, b: V, op: char) -> Eval<V> {
    let int = |x: i64, y: i64| -> Eval<V> {
        let r = match op {
            '*' => x.checked_mul(y).ok_or("integer overflow")?,
            _ if y == 0 => return Err("division by zero".to_owned()),
            '/' => x / y,
            _ => x % y,
        };
        Ok(V::Int(r))
    };
    let float = |x: f64, y: f64| match op {
        '*' => x * y,
        '/' => x / y,
        _ => x % y,
    };
    Ok(match (a, b) {
        (V::Null, _) | (_, V::Null) => V::Null,
        (V::Int(x), V::Int(y)) => int(x, y)?,
        (V::Duration(s), n) | (n, V::Duration(s)) if op == '*' && n.num().is_some() => {
            V::Duration(s * n.num().unwrap_or_default())
        }
        (V::Duration(s), n) if op == '/' && n.num().is_some() => {
            V::Duration(s / n.num().unwrap_or_default())
        }
        (a, b) => match (a.num(), b.num()) {
            (Some(x), Some(y)) => V::Float(float(x, y)),
            _ => {
                return Err(format!(
                    "cannot apply `{op}` to {} and {}",
                    a.kind(),
                    b.kind()
                ));
            }
        },
    })
}

/// A stored field value, typed by its field's kind.
pub(crate) fn from_json(raw: &serde_json::Value, kind: &FieldKind) -> V {
    use serde_json::Value as J;
    if let J::Array(items) = raw {
        return V::List(items.iter().map(|i| from_json(i, kind)).collect());
    }
    match (kind, raw) {
        (_, J::Null) => V::Null,
        (FieldKind::Date, J::String(s)) => time::date(s).map_or(V::Str(s.clone()), V::Date),
        (FieldKind::Datetime, J::String(s)) => {
            time::datetime(s).map_or(V::Str(s.clone()), V::Datetime)
        }
        (FieldKind::DateRange { of }, J::Object(m)) => V::Record(
            m.iter()
                .map(|(k, v)| {
                    // Either form reads by its shape: a date is ten characters.
                    let datetime = match of {
                        RangeOf::Date => false,
                        RangeOf::Datetime => true,
                        RangeOf::DateOrDatetime => v.as_str().is_some_and(|s| s.len() != 10),
                    };
                    let kind = if datetime {
                        FieldKind::Datetime
                    } else {
                        FieldKind::Date
                    };
                    (k.clone(), from_json(v, &kind))
                })
                .collect(),
        ),
        (FieldKind::List { item }, v) => from_json(v, &item.kind),
        // A dict's entries, each by its own definition (types T5).
        (FieldKind::Dict { keys, values }, J::Object(m)) => V::Record(
            m.iter()
                .map(|(k, v)| {
                    let def = keys.iter().find(|d| &d.name == k).or(match values {
                        schema::DictValues::Of(d) => Some(d.as_ref()),
                        _ => None,
                    });
                    (
                        k.clone(),
                        def.map_or_else(|| plain(v), |d| from_json(v, &d.kind)),
                    )
                })
                .collect(),
        ),
        (FieldKind::Reference(_), J::String(s)) => V::Ref(s.clone()),
        (FieldKind::Float, J::Number(n)) => V::Float(n.as_f64().unwrap_or_default()),
        (_, v) => plain(v),
    }
}

/// A JSON value with no field kind to go by.
pub(crate) fn plain(raw: &serde_json::Value) -> V {
    use serde_json::Value as J;
    match raw {
        J::Null => V::Null,
        J::Bool(b) => V::Bool(*b),
        J::Number(n) => n
            .as_i64()
            .map(V::Int)
            .unwrap_or_else(|| V::Float(n.as_f64().unwrap_or_default())),
        J::String(s) => V::Str(s.clone()),
        J::Array(a) => V::List(a.iter().map(plain).collect()),
        J::Object(m) => V::Record(m.iter().map(|(k, v)| (k.clone(), plain(v))).collect()),
    }
}

/// Dates and datetimes as Unix time.
pub(crate) mod time {
    /// Days since 1970-01-01 of `YYYY-MM-DD`.
    pub fn date(s: &str) -> Option<i64> {
        let b = s.as_bytes();
        if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
            return None;
        }
        let (y, m, d) = (digits(&s[0..4])?, digits(&s[5..7])?, digits(&s[8..10])?);
        if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
            return None;
        }
        Some(days_from_civil(y, m, d))
    }

    /// Seconds since the Unix epoch of `YYYY-MM-DDTHH:MM:SS[.fff](Z|±HH:MM)`.
    pub fn datetime(s: &str) -> Option<f64> {
        if s.len() < 20 || s.as_bytes()[10] != b'T' {
            return None;
        }
        let days = date(s.get(..10)?)?;
        let t = &s[11..];
        let (h, mi, sec) = (
            digits(t.get(0..2)?)?,
            digits(t.get(3..5)?)?,
            digits(t.get(6..8)?)?,
        );
        let mut rest = t.get(8..)?;
        let mut frac = 0.0;
        if let Some(after) = rest.strip_prefix('.') {
            let n = after
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(after.len());
            frac = format!("0.{}", &after[..n]).parse().ok()?;
            rest = &after[n..];
        }
        let offset = match rest.as_bytes().first()? {
            b'Z' => 0,
            sign @ (b'+' | b'-') => {
                let o = &rest[1..];
                let secs = digits(o.get(0..2)?)? * 3600 + digits(o.get(3..5)?)? * 60;
                if *sign == b'+' { secs } else { -secs }
            }
            _ => return None,
        };
        Some((days * 86_400 + h * 3600 + mi * 60 + sec - offset) as f64 + frac)
    }

    fn digits(s: &str) -> Option<i64> {
        if s.bytes().all(|c| c.is_ascii_digit()) {
            s.parse().ok()
        } else {
            None
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
}

/// A sort key: a value reduced to something with a total order that survives a cursor.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "k", content = "v")]
pub(crate) enum Key {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    List(Vec<Key>),
    /// Missing. Always sorts last, in either direction.
    Null,
}

impl Key {
    pub fn of(v: &V) -> Key {
        match v {
            V::Null => Key::Null,
            V::Bool(b) => Key::Bool(*b),
            V::Int(n) => Key::Int(*n),
            V::Float(x) | V::Duration(x) => Key::Float(*x),
            V::Date(_) | V::Datetime(_) => Key::Float(v.time().unwrap_or_default()),
            V::Str(s) | V::Ref(s) => Key::Str(s.clone()),
            V::List(l) => Key::List(l.iter().map(Key::of).collect()),
            V::Record(r) => Key::List(r.iter().map(|(_, v)| Key::of(v)).collect()),
        }
    }

    fn rank(&self) -> u8 {
        match self {
            Key::Bool(_) => 0,
            Key::Int(_) | Key::Float(_) => 1,
            Key::Str(_) => 2,
            Key::List(_) => 3,
            Key::Null => 4,
        }
    }

    /// The order of two present values. Missing values are handled by the caller.
    pub fn cmp(&self, other: &Key) -> Ordering {
        match (self, other) {
            (Key::Bool(a), Key::Bool(b)) => a.cmp(b),
            (Key::Int(a), Key::Int(b)) => a.cmp(b),
            (Key::Int(a), Key::Float(b)) => (*a as f64).total_cmp(b),
            (Key::Float(a), Key::Int(b)) => a.total_cmp(&(*b as f64)),
            (Key::Float(a), Key::Float(b)) => a.total_cmp(b),
            (Key::Str(a), Key::Str(b)) => a.cmp(b),
            (Key::List(a), Key::List(b)) => {
                for (x, y) in a.iter().zip(b) {
                    let o = x.ordered(y, false);
                    if o != Ordering::Equal {
                        return o;
                    }
                }
                a.len().cmp(&b.len())
            }
            _ => self.rank().cmp(&other.rank()),
        }
    }

    /// The order with `desc` applied, keeping missing values last.
    pub fn ordered(&self, other: &Key, desc: bool) -> Ordering {
        match (self, other) {
            (Key::Null, Key::Null) => Ordering::Equal,
            (Key::Null, _) => Ordering::Greater,
            (_, Key::Null) => Ordering::Less,
            _ if desc => other.cmp(self),
            _ => self.cmp(other),
        }
    }
}
