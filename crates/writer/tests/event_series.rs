//! Event series (event-series E1, E4, E5): the built-in type, the rule check, the carried
//! fields and the occurrences the home server makes for a series kept in the vault.
//!
//! The clock starts at Monday 2 March 2026, 00:00 UTC, so a vault-only series that starts that
//! day has occurrences from then to 1 March 2027.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use serde_json::{Value, json};
use state::{Author, State};
use store::Vault;
use writer::{EVENT_SERIES_TYPE, EVENT_TYPE, Error, Input, Upkeep, Writer};

/// 2026-03-02T00:00:00Z.
const START_MS: i64 = 1_772_409_600_000;
const DAY_MS: i64 = 86_400_000;

struct Clock(AtomicI64);

impl state::Clock for Clock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

fn open(root: &Path, upkeep: Upkeep) -> (Writer, Arc<Clock>) {
    let vault = Vault::init(root).unwrap();
    let state = State::open(&vault.server_dir().join("state.db"), None).unwrap();
    let clock = Arc::new(Clock(AtomicI64::new(START_MS)));
    let w = Writer::open(vault, state)
        .unwrap()
        .with_clock(clock.clone());
    w.upgrade_shipped_defaults(upkeep).unwrap();
    (w, clock)
}

fn user() -> Author {
    Author::Session("tester".into())
}

/// Add field `room`, a string, to both `event` and `event_series`, as a carried field.
fn add_room(w: &Writer) {
    for ty in [EVENT_TYPE, EVENT_SERIES_TYPE] {
        let text = w.read_type(ty).unwrap();
        let text = format!("{text}\nfield room {{\n  kind  = \"string\"\n  label = \"Room\"\n}}\n");
        w.put_type(ty, &text).unwrap();
    }
}

fn create(w: &Writer, type_name: &str, note: Value) -> String {
    w.create(type_name, &Input::Json(note), &user()).unwrap().id
}

fn weekly(extra: Value) -> Value {
    let mut fields = json!({
        "title": "Standup",
        "rule": ["RRULE:FREQ=WEEKLY;BYDAY=MO"],
        "starts": { "start": "2026-03-02", "end": "2026-03-02" },
        "location": "Office",
    });
    for (k, v) in extra.as_object().unwrap() {
        fields[k] = v.clone();
    }
    json!({ "fields": fields })
}

/// The fields of note `id`, as JSON.
fn fields(w: &Writer, id: &str) -> Value {
    writer::doc_to_json(&w.read(id).unwrap().doc)["fields"].clone()
}

/// The occurrences linked to series `id`, by their `when.start`.
fn occurrences(w: &Writer, id: &str) -> Vec<(String, String, Value)> {
    let mut out = Vec::new();
    for s in w.list(Some(EVENT_TYPE)).unwrap() {
        let f = fields(w, &s.id);
        if f["series_ref"] == json!(id) {
            out.push((
                f["when"]["start"].as_str().unwrap_or("").to_owned(),
                s.id.clone(),
                f,
            ));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn edit(w: &Writer, id: &str, changes: Value) {
    let note = w.read(id).unwrap();
    let mut json = writer::doc_to_json(&note.doc);
    for (k, v) in changes.as_object().unwrap() {
        if v.is_null() {
            json["fields"].as_object_mut().unwrap().remove(k);
        } else {
            json["fields"][k] = v.clone();
        }
    }
    w.update(id, &Input::Json(json), Some(&note.version), &user())
        .unwrap();
}

#[test]
fn a_vault_gets_the_series_type_and_event_gains_its_series_fields() {
    let dir = tempfile::tempdir().unwrap();
    let (w, _) = open(dir.path(), Upkeep::Home);
    assert!(w.is_built_in(EVENT_SERIES_TYPE));
    let schema = w.schema();
    let series = schema.type_def(EVENT_SERIES_TYPE).unwrap();
    for f in [
        "rule",
        "starts",
        "timezone",
        "calendar",
        "google_ref",
        "snapshot",
    ] {
        assert!(series.field(f).is_some(), "event_series lacks {f}");
    }
    let event = schema.type_def(EVENT_TYPE).unwrap();
    for (f, hidden, computed) in [
        ("series_ref", true, false),
        ("series_key", true, false),
        ("series_overrides", true, true),
    ] {
        let def = event.field(f).unwrap();
        assert_eq!((def.hidden, def.computed), (hidden, computed), "{f}");
    }
    // Nothing is carried until both types have a field of the user's own.
    assert!(writer::carried_fields(&schema).is_empty());
    add_room(&w);
    let carried: Vec<String> = writer::carried_fields(&w.schema())
        .into_iter()
        .map(|f| f.name)
        .collect();
    assert_eq!(carried, ["room"]);
}

#[test]
fn a_rule_that_does_not_parse_is_refused_on_every_writer() {
    for upkeep in [Upkeep::Home, Upkeep::Device] {
        let dir = tempfile::tempdir().unwrap();
        let (w, _) = open(dir.path(), upkeep);
        let note = weekly(json!({ "rule": ["RRULE:FREQ=FORTNIGHTLY"] }));
        let err = w
            .create(EVENT_SERIES_TYPE, &Input::Json(note), &user())
            .unwrap_err();
        let Error::Invalid(invalid) = err else {
            panic!("{err}");
        };
        assert_eq!(invalid.issues[0].at, "rule");
        assert!(
            invalid.issues[0]
                .message
                .contains("is not a recurrence rule"),
            "{}",
            invalid.issues[0].message
        );
        let note = weekly(json!({ "rule": ["every monday"] }));
        assert!(
            w.create(EVENT_SERIES_TYPE, &Input::Json(note), &user())
                .is_err()
        );
    }
}

#[test]
fn a_rule_google_sent_is_kept_as_it_is_and_a_typed_one_is_still_checked() {
    // RFC 7529's RSCALE, which the rule library does not read.
    let odd = json!(["RRULE:FREQ=WEEKLY;BYDAY=MO;RSCALE=GREGORIAN"]);
    let sync = Author::Task("google_calendar".into());
    for upkeep in [Upkeep::Home, Upkeep::Device] {
        let dir = tempfile::tempdir().unwrap();
        let (w, _) = open(dir.path(), upkeep);
        let google = weekly(json!({ "rule": odd, "calendar": "primary", "google_ref": "m1" }));
        let id = w
            .create(EVENT_SERIES_TYPE, &Input::Json(google.clone()), &sync)
            .unwrap()
            .id;
        assert_eq!(fields(&w, &id)["rule"], odd);
        // Any writer may change the rest of the series and keep the rule.
        edit(&w, &id, json!({ "title": "Team standup" }));
        assert_eq!(fields(&w, &id)["rule"], odd);

        // A rule typed in the vault is checked, on a Google series too.
        let note = w.read(&id).unwrap();
        let mut json = writer::doc_to_json(&note.doc);
        json["fields"]["rule"] = json!(["RRULE:FREQ=WEEKLY;RSCALE=JULIAN"]);
        let err = w
            .update(&id, &Input::Json(json), Some(&note.version), &user())
            .unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{err}");
        let err = w
            .create(EVENT_SERIES_TYPE, &Input::Json(google), &user())
            .unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{err}");
        // A series kept in the vault takes no rule that does not read, from the sync either.
        let local = weekly(json!({ "rule": odd }));
        let err = w
            .create(EVENT_SERIES_TYPE, &Input::Json(local), &sync)
            .unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{err}");
    }
}

#[test]
fn a_vault_only_series_gets_its_occurrences_a_year_ahead_in_the_same_commit() {
    let dir = tempfile::tempdir().unwrap();
    let (w, _) = open(dir.path(), Upkeep::Home);
    add_room(&w);
    let saved = w
        .create(
            EVENT_SERIES_TYPE,
            &Input::Json(weekly(json!({ "room": "Blue" }))),
            &user(),
        )
        .unwrap();
    let id = saved.id.clone();
    let occ = occurrences(&w, &id);
    // Every Monday from 2 March 2026 to 1 March 2027.
    assert_eq!(occ.len(), 53);
    assert_eq!(
        saved.also.len(),
        53,
        "one commit makes the series and its occurrences"
    );
    assert_eq!(occ[0].0, "2026-03-02");
    assert_eq!(occ[52].0, "2027-03-01");
    let f = &occ[0].2;
    assert_eq!(f["title"], "Standup");
    assert_eq!(f["location"], "Office");
    assert_eq!(f["room"], "Blue");
    assert_eq!(f["when"]["end"], "2026-03-02");
    assert_eq!(f["series_key"], json!(format!("{id}_20260302")));
    assert!(f.get("google_ref").is_none());
    assert!(f.get("series_overrides").is_none());
    // The occurrences are the server's.
    let history = w.history(&occ[0].1).unwrap();
    assert_eq!(history[0].author, Author::Task("server".into()));
}

#[test]
fn a_series_that_started_long_ago_has_occurrences_from_30_days_back() {
    let dir = tempfile::tempdir().unwrap();
    let (w, _) = open(dir.path(), Upkeep::Home);
    let id = create(
        &w,
        EVENT_SERIES_TYPE,
        weekly(json!({ "starts": { "start": "2025-01-06", "end": "2025-01-06" } })),
    );
    let occ = occurrences(&w, &id);
    // 30 days before 2 March 2026 is 31 January; the first Monday after is 2 February.
    assert_eq!(occ[0].0, "2026-02-02");
    assert_eq!(occ.last().unwrap().0, "2027-03-01");
}

#[test]
fn a_timed_series_in_a_zone_makes_times_in_that_zone() {
    let dir = tempfile::tempdir().unwrap();
    let (w, _) = open(dir.path(), Upkeep::Home);
    let id = create(
        &w,
        EVENT_SERIES_TYPE,
        weekly(json!({
            "rule": ["RRULE:FREQ=WEEKLY;BYDAY=MO;COUNT=6"],
            "starts": { "start": "2026-03-02T09:30:00+11:00", "end": "2026-03-02T10:00:00+11:00" },
            "timezone": "Australia/Melbourne",
        })),
    );
    let occ = occurrences(&w, &id);
    let starts: Vec<&str> = occ.iter().map(|o| o.0.as_str()).collect();
    // Melbourne leaves daylight saving on 5 April.
    assert_eq!(
        starts,
        [
            "2026-03-02T09:30:00+11:00",
            "2026-03-09T09:30:00+11:00",
            "2026-03-16T09:30:00+11:00",
            "2026-03-23T09:30:00+11:00",
            "2026-03-30T09:30:00+11:00",
            "2026-04-06T09:30:00+10:00",
        ]
    );
    assert_eq!(occ[5].2["when"]["end"], "2026-04-06T10:00:00+10:00");
    assert_eq!(
        occ[0].2["series_key"],
        json!(format!("{id}_20260301T223000Z"))
    );
}

#[test]
fn a_carried_field_reaches_every_occurrence_but_one_changed_by_hand() {
    let dir = tempfile::tempdir().unwrap();
    let (w, _) = open(dir.path(), Upkeep::Home);
    add_room(&w);
    let id = create(&w, EVENT_SERIES_TYPE, weekly(json!({})));
    let occ = occurrences(&w, &id);
    assert!(occ.iter().all(|o| o.2.get("room").is_none()));

    edit(&w, &id, json!({ "room": "Blue" }));
    let occ = occurrences(&w, &id);
    assert!(occ.iter().all(|o| o.2["room"] == "Blue"));

    // By hand: the occurrence keeps its own value through the next series change.
    let second = occ[1].1.clone();
    edit(&w, &second, json!({ "room": "Red" }));
    assert_eq!(fields(&w, &second)["series_overrides"], json!(["room"]));
    edit(&w, &id, json!({ "room": "Green" }));
    let occ = occurrences(&w, &id);
    assert_eq!(occ[0].2["room"], "Green");
    assert_eq!(occ[1].2["room"], "Red");
    assert_eq!(occ[2].2["room"], "Green");

    // Set back to the series' value, the override goes, so the next change reaches it.
    edit(&w, &second, json!({ "room": "Green" }));
    assert!(fields(&w, &second).get("series_overrides").is_none());
    edit(&w, &id, json!({ "room": "Teal" }));
    assert_eq!(fields(&w, &second)["room"], "Teal");

    // Cleared, it takes the series' value back at once.
    edit(&w, &second, json!({ "room": "Red" }));
    edit(&w, &second, json!({ "room": null }));
    let f = fields(&w, &second);
    assert_eq!(f["room"], "Teal");
    assert!(f.get("series_overrides").is_none());
}

#[test]
fn a_google_series_carries_custom_fields_but_not_google_fields() {
    let dir = tempfile::tempdir().unwrap();
    let (w, _) = open(dir.path(), Upkeep::Home);
    add_room(&w);
    let id = create(
        &w,
        EVENT_SERIES_TYPE,
        weekly(json!({ "calendar": "primary", "google_ref": "master1" })),
    );
    // A Google series makes no occurrences of its own: the calendar sync brings them.
    assert!(occurrences(&w, &id).is_empty());
    let occ = create(
        &w,
        EVENT_TYPE,
        json!({ "fields": {
            "title": "Standup",
            "when": { "start": "2026-03-09", "end": "2026-03-09" },
            "location": "Office",
            "google_ref": "master1_20260309",
            "series_ref": id,
        }}),
    );
    edit(
        &w,
        &id,
        json!({ "room": "Blue", "location": "Lab", "title": "Daily" }),
    );
    let f = fields(&w, &occ);
    assert_eq!(f["room"], "Blue");
    assert_eq!(f["location"], "Office");
    assert_eq!(f["title"], "Standup");
    // An occurrence linked later takes the series' carried value.
    let late = create(
        &w,
        EVENT_TYPE,
        json!({ "fields": {
            "title": "Standup",
            "when": { "start": "2026-03-16", "end": "2026-03-16" },
            "google_ref": "master1_20260316",
        }}),
    );
    edit(&w, &late, json!({ "series_ref": id }));
    assert_eq!(fields(&w, &late)["room"], "Blue");
}

#[test]
fn a_rule_change_makes_the_occurrences_again_and_keeps_those_changed_by_hand() {
    let dir = tempfile::tempdir().unwrap();
    let (w, _) = open(dir.path(), Upkeep::Home);
    add_room(&w);
    let id = create(&w, EVENT_SERIES_TYPE, weekly(json!({})));
    let occ = occurrences(&w, &id);
    let renamed = occ[1].1.clone();
    edit(&w, &renamed, json!({ "title": "Standup, with the board" }));
    let roomed = occ[2].1.clone();
    edit(&w, &roomed, json!({ "room": "Red" }));
    let plain = occ[3].1.clone();

    edit(&w, &id, json!({ "rule": ["RRULE:FREQ=WEEKLY;BYDAY=WE"] }));
    let occ = occurrences(&w, &id);
    // Changed by hand, the old Mondays stay, still linked; the plain one went to the trash.
    assert!(occ.iter().any(|o| o.1 == renamed));
    assert!(occ.iter().any(|o| o.1 == roomed));
    assert!(w.read(&plain).is_err());
    assert!(w.trash().unwrap().iter().any(|t| t.note_id == plain));
    // Every Wednesday from 4 March 2026 to 24 February 2027, once each.
    let made: Vec<&str> = occ
        .iter()
        .filter(|o| o.1 != renamed && o.1 != roomed)
        .map(|o| o.0.as_str())
        .collect();
    let mut wednesdays = Vec::new();
    let mut d = chrono::NaiveDate::from_ymd_opt(2026, 3, 4).unwrap();
    while d < chrono::NaiveDate::from_ymd_opt(2027, 3, 2).unwrap() {
        wednesdays.push(d.format("%Y-%m-%d").to_string());
        d += chrono::Duration::days(7);
    }
    assert_eq!(made, wednesdays);

    // A new location reaches every occurrence still holding the old one.
    edit(&w, &id, json!({ "location": "Lab", "title": "Sync" }));
    let occ = occurrences(&w, &id);
    // The Mondays kept are left as they are.
    let kept = occ.iter().find(|o| o.1 == renamed).unwrap();
    assert_eq!(kept.2["title"], "Standup, with the board");
    assert_eq!(kept.2["location"], "Office");
    let made: Vec<_> = occ
        .iter()
        .filter(|o| o.1 != renamed && o.1 != roomed)
        .collect();
    assert!(
        made.iter()
            .all(|o| o.2["title"] == "Sync" && o.2["location"] == "Lab")
    );
}

#[test]
fn deleting_an_occurrence_adds_an_exdate_so_it_is_not_made_again() {
    let dir = tempfile::tempdir().unwrap();
    let (w, _) = open(dir.path(), Upkeep::Home);
    let id = create(&w, EVENT_SERIES_TYPE, weekly(json!({})));
    let occ = occurrences(&w, &id);
    let gone = occ[2].1.clone();
    let version = w.read(&gone).unwrap().version;
    w.delete(&gone, Some(&version), &user()).unwrap();
    assert_eq!(
        fields(&w, &id)["rule"],
        json!(["RRULE:FREQ=WEEKLY;BYDAY=MO", "EXDATE;VALUE=DATE:20260316"])
    );
    let occ = occurrences(&w, &id);
    assert_eq!(occ.len(), 52);
    assert!(occ.iter().all(|o| o.0 != "2026-03-16"));
    // A later change to the series makes it no more.
    edit(&w, &id, json!({ "location": "Lab" }));
    assert_eq!(occurrences(&w, &id).len(), 52);
    // Deleting the series deletes its occurrences.
    let version = w.read(&id).unwrap().version;
    w.delete(&id, Some(&version), &user()).unwrap();
    assert!(occurrences(&w, &id).is_empty());
    assert!(w.list(Some(EVENT_TYPE)).unwrap().is_empty());
}

#[test]
fn the_window_rolls_forward_as_the_days_pass() {
    let dir = tempfile::tempdir().unwrap();
    let (w, clock) = open(dir.path(), Upkeep::Home);
    let id = create(&w, EVENT_SERIES_TYPE, weekly(json!({})));
    assert_eq!(occurrences(&w, &id).len(), 53);
    assert_eq!(w.roll_series().unwrap(), 0, "nothing new on the same day");
    clock.0.fetch_add(7 * DAY_MS, Ordering::SeqCst);
    assert_eq!(w.roll_series().unwrap(), 1);
    let occ = occurrences(&w, &id);
    assert_eq!(occ.len(), 54);
    assert_eq!(occ.last().unwrap().0, "2027-03-08");
    // The past ones stay.
    assert_eq!(occ[0].0, "2026-03-02");
}

#[test]
fn rolling_forward_skips_a_series_whose_rule_was_broken_by_hand() {
    let dir = tempfile::tempdir().unwrap();
    let (w, clock) = open(dir.path(), Upkeep::Home);
    let broken = create(&w, EVENT_SERIES_TYPE, weekly(json!({})));
    let fine = create(&w, EVENT_SERIES_TYPE, weekly(json!({})));
    let path = dir
        .path()
        .join(format!("notes/{EVENT_SERIES_TYPE}/{broken}.wcl"));
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("FREQ=WEEKLY;BYDAY=MO"), "{text}");
    std::fs::write(
        &path,
        text.replace("FREQ=WEEKLY;BYDAY=MO", "FREQ=FORTNIGHTLY"),
    )
    .unwrap();
    clock.0.fetch_add(7 * DAY_MS, Ordering::SeqCst);
    assert_eq!(w.roll_series().unwrap(), 1, "only the series that reads");
    assert_eq!(occurrences(&w, &fine).len(), 54);
    assert_eq!(occurrences(&w, &broken).len(), 53);
}

#[test]
fn a_device_makes_no_occurrences_and_carries_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (w, _) = open(dir.path(), Upkeep::Device);
    let id = create(&w, EVENT_SERIES_TYPE, weekly(json!({})));
    assert!(occurrences(&w, &id).is_empty());
    assert_eq!(w.roll_series().unwrap(), 0);
}

#[test]
fn giving_a_vault_only_series_a_calendar_hands_its_occurrences_to_google() {
    let dir = tempfile::tempdir().unwrap();
    let (w, _) = open(dir.path(), Upkeep::Home);
    let id = create(&w, EVENT_SERIES_TYPE, weekly(json!({})));
    assert_eq!(occurrences(&w, &id).len(), 53);
    edit(&w, &id, json!({ "calendar": "primary" }));
    assert!(occurrences(&w, &id).is_empty());
}
