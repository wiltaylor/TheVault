//! The fake Google server's Calendar API over HTTP: calendarList, events list with sync tokens
//! and 410, instances, get, insert, patch with `If-Match` and delete (google-sync D9, D16).

use reqwest::blocking::{Client, RequestBuilder};
use serde_json::{Value, json};
use testkit::FakeGoogle;
use testkit::fake_google::{DEFAULT_CLIENT_ID, DEFAULT_CLIENT_SECRET, DEFAULT_REFRESH_TOKEN};

struct Cal {
    google: FakeGoogle,
    token: String,
    client: Client,
}

impl Cal {
    fn start() -> Cal {
        let google = FakeGoogle::builder()
            .calendar("team@group.calendar.google.com", "Team")
            .start();
        let client = Client::new();
        let res: Value = client
            .post(google.endpoints()["token"].as_str().unwrap())
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", DEFAULT_CLIENT_ID),
                ("client_secret", DEFAULT_CLIENT_SECRET),
                ("refresh_token", DEFAULT_REFRESH_TOKEN),
            ])
            .send()
            .unwrap()
            .json()
            .unwrap();
        let token = res["access_token"].as_str().unwrap().to_owned();
        Cal {
            google,
            token,
            client,
        }
    }

    fn req(&self, method: &str, path: &str) -> RequestBuilder {
        let url = format!(
            "{}{path}",
            self.google.endpoints()["calendar"].as_str().unwrap()
        );
        self.client
            .request(method.parse().unwrap(), url)
            .bearer_auth(&self.token)
    }

    fn send(&self, b: RequestBuilder) -> (u16, Value) {
        let res = b.send().unwrap();
        let status = res.status().as_u16();
        (status, res.json().unwrap_or(Value::Null))
    }

    fn get(&self, path: &str) -> (u16, Value) {
        self.send(self.req("GET", path))
    }
}

fn ids(body: &Value) -> Vec<&str> {
    body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap())
        .collect()
}

#[test]
fn calendar_requests_need_an_issued_access_token() {
    let cal = Cal::start();
    let res = cal
        .client
        .get(format!(
            "{}/users/me/calendarList",
            cal.google.endpoints()["calendar"].as_str().unwrap()
        ))
        .bearer_auth("made-up")
        .send()
        .unwrap();
    assert_eq!(res.status().as_u16(), 401);
    let (status, list) = cal.get("/users/me/calendarList");
    assert_eq!(status, 200);
    assert_eq!(ids(&list), ["primary", "team@group.calendar.google.com"]);
    assert_eq!(list["items"][0]["primary"], true);
    assert_eq!(cal.get("/calendars/nope/events").0, 404);
    // A calendar id is percent-decoded from the path.
    assert_eq!(
        cal.get("/calendars/team%40group.calendar.google.com/events")
            .0,
        200
    );
}

#[test]
fn events_list_pages_issues_sync_tokens_and_answers_410_once_they_expire() {
    let cal = Cal::start();
    for n in 1..=3 {
        cal.google.put_event(
            "primary",
            json!({ "id": format!("e{n}"), "summary": "x", "start": { "date": "2026-03-01" }, "end": { "date": "2026-03-02" } }),
        );
    }
    let (_, first) = cal.get("/calendars/primary/events?maxResults=2");
    assert_eq!(ids(&first), ["e1", "e2"]);
    assert!(first.get("nextSyncToken").is_none());
    let page = first["nextPageToken"].as_str().unwrap();
    let (_, second) = cal.get(&format!(
        "/calendars/primary/events?maxResults=2&pageToken={page}"
    ));
    assert_eq!(ids(&second), ["e3"]);
    let token = second["nextSyncToken"].as_str().unwrap().to_owned();

    // Only changes since the token, a cancel included.
    cal.google.cancel_event("primary", "e2");
    cal.google.put_event(
        "primary",
        json!({ "id": "e4", "start": { "date": "2026-03-01" }, "end": { "date": "2026-03-02" } }),
    );
    let (_, changes) = cal.get(&format!("/calendars/primary/events?syncToken={token}"));
    assert_eq!(ids(&changes), ["e2", "e4"]);
    assert_eq!(changes["items"][0]["status"], "cancelled");
    // A full listing leaves the cancelled event out.
    let (_, full) = cal.get("/calendars/primary/events");
    assert_eq!(ids(&full), ["e1", "e3", "e4"]);

    cal.google.expire_sync_tokens();
    let (status, body) = cal.get(&format!("/calendars/primary/events?syncToken={token}"));
    assert_eq!(status, 410, "{body}");
    assert_eq!(cal.get("/calendars/primary/events?syncToken=junk").0, 410);
}

#[test]
fn instances_expand_a_series_in_a_window_with_changed_and_cancelled_occurrences() {
    let cal = Cal::start();
    cal.google.put_event(
        "primary",
        json!({
            "id": "s",
            "summary": "Standup",
            "start": { "dateTime": "2026-03-02T09:00:00+11:00" },
            "end": { "dateTime": "2026-03-02T09:15:00+11:00" },
            "recurrence": ["RRULE:FREQ=DAILY;INTERVAL=2;UNTIL=20260310T000000Z"],
        }),
    );
    let (_, all) = cal.get("/calendars/primary/events/s/instances");
    assert_eq!(
        ids(&all),
        [
            "s_20260301T220000Z",
            "s_20260303T220000Z",
            "s_20260305T220000Z",
            "s_20260307T220000Z",
            "s_20260309T220000Z"
        ]
    );
    assert_eq!(
        all["items"][1]["start"]["dateTime"],
        "2026-03-04T09:00:00+11:00"
    );
    assert_eq!(all["items"][1]["recurringEventId"], "s");
    assert_eq!(
        all["items"][1]["originalStartTime"]["dateTime"],
        "2026-03-04T09:00:00+11:00"
    );

    cal.google.change_event(
        "primary",
        "s_20260303T220000Z",
        json!({ "summary": "Moved" }),
    );
    cal.google.cancel_event("primary", "s_20260305T220000Z");
    let (_, window) = cal.get(
        "/calendars/primary/events/s/instances?timeMin=2026-03-03T00:00:00Z&timeMax=2026-03-09T00:00:00Z",
    );
    assert_eq!(ids(&window), ["s_20260303T220000Z", "s_20260307T220000Z"]);
    assert_eq!(window["items"][0]["summary"], "Moved");
    // A get of a cancelled occurrence answers it, cancelled.
    let (status, got) = cal.get("/calendars/primary/events/s_20260305T220000Z");
    assert_eq!((status, got["status"].as_str()), (200, Some("cancelled")));
    assert_eq!(cal.get("/calendars/primary/events/never").0, 404);
}

#[test]
fn insert_patch_with_if_match_and_delete() {
    let cal = Cal::start();
    let (status, made) = cal.send(cal.req("POST", "/calendars/primary/events").json(&json!({
        "summary": "Lunch",
        "start": { "date": "2026-03-04" },
        "end": { "date": "2026-03-05" },
    })));
    assert_eq!(status, 200);
    let id = made["id"].as_str().unwrap().to_owned();
    let etag = made["etag"].as_str().unwrap().to_owned();
    assert!(etag.starts_with('"') && etag.ends_with('"'), "{etag}");
    assert_eq!(made["status"], "confirmed");

    let path = format!("/calendars/primary/events/{id}");
    let (status, patched) = cal.send(
        cal.req("PATCH", &path)
            .header("if-match", &etag)
            .json(&json!({ "location": "Cafe" })),
    );
    assert_eq!(status, 200);
    assert_eq!(patched["summary"], "Lunch");
    assert_eq!(patched["location"], "Cafe");
    assert_ne!(patched["etag"], json!(etag));
    // The old etag is stale now.
    let (status, _) = cal.send(
        cal.req("PATCH", &path)
            .header("if-match", &etag)
            .json(&json!({ "location": "Elsewhere" })),
    );
    assert_eq!(status, 412);
    assert_eq!(
        cal.google.event("primary", &id).unwrap()["location"],
        "Cafe"
    );

    assert_eq!(cal.send(cal.req("DELETE", &path)).0, 204);
    assert_eq!(cal.get(&path).1["status"], "cancelled");
    assert_eq!(cal.send(cal.req("DELETE", &path)).0, 410);
    let inserts = cal
        .google
        .requests_to("/calendar/v3/calendars/primary/events");
    assert_eq!(inserts[0].json()["summary"], "Lunch");
}

#[test]
fn a_calendar_delay_holds_requests_until_it_is_cleared() {
    use std::time::{Duration, Instant};
    let cal = Cal::start();
    cal.google.set_calendar_delay(Some(Duration::from_secs(60)));
    let held = std::thread::scope(|s| {
        let began = Instant::now();
        let req = s.spawn(|| cal.get("/users/me/calendarList").0);
        std::thread::sleep(Duration::from_millis(300));
        assert!(!req.is_finished(), "the request answered under the delay");
        cal.google.set_calendar_delay(None);
        let status = req.join().unwrap();
        (status, began.elapsed())
    });
    assert_eq!(held.0, 200);
    assert!(
        held.1 < Duration::from_secs(10),
        "released after {:?}",
        held.1
    );
    let began = Instant::now();
    assert_eq!(cal.get("/users/me/calendarList").0, 200);
    assert!(began.elapsed() < Duration::from_secs(1));
}
