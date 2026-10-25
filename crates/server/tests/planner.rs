//! S1 with S3 checks: the planner's built-in period note types and the note for a day, an ISO
//! week, a month, a quarter or a year.

use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use reqwest::header::AUTHORIZATION;
use serde_json::Value;
use testkit::TestServer;

const PERIOD_TYPES: [&str; 4] = ["weeklynote", "monthlynote", "quarterlynote", "yearlynote"];

struct Api {
    server: TestServer,
    client: Client,
}

impl Api {
    fn new(server: TestServer) -> Self {
        Api {
            server,
            client: Client::new(),
        }
    }

    fn send(&self, method: &str, path: &str) -> Response {
        self.client
            .request(method.parse().unwrap(), self.server.url(path))
            .header(
                AUTHORIZATION,
                format!("Bearer {}", self.server.admin_token()),
            )
            .send()
            .unwrap()
    }

    fn type_file(&self, name: &str) -> String {
        std::fs::read_to_string(self.server.vault_path().join(format!("types/{name}.wcl"))).unwrap()
    }
}

fn expect(res: Response, status: StatusCode) -> Value {
    let got = res.status();
    let text = res.text().unwrap();
    assert_eq!(got, status, "{text}");
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

#[test]
fn every_vault_has_the_period_note_types_built_in() {
    let api = Api::new(TestServer::start());
    let types = expect(api.send("GET", "/api/types"), StatusCode::OK);
    for name in PERIOD_TYPES {
        let ty = types["types"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == name)
            .unwrap_or_else(|| panic!("no {name} in {types}"));
        assert_eq!(ty["builtin"], true, "{name}");
        let file = api.type_file(name);
        assert!(file.contains("field date"), "{file}");
        assert!(file.contains("period_calendar"), "{file}");
        let body = expect(
            api.send("DELETE", &format!("/api/types/{name}")),
            StatusCode::CONFLICT,
        );
        assert_eq!(body["error"], "built_in_type");
    }
}

#[test]
fn any_day_of_a_period_finds_or_creates_the_one_note_of_its_period() {
    let api = Api::new(TestServer::start());
    // (period, a day inside it, another day inside it, the period's start, the title, the type)
    let cases = [
        (
            "week",
            "2026-10-25",
            "2026-10-19",
            "2026-10-19",
            "Week 43, 2026",
            "weeklynote",
        ),
        // 2021-01-01 is in week 53 of 2020, which starts on 2020-12-28.
        (
            "week",
            "2021-01-01",
            "2020-12-31",
            "2020-12-28",
            "Week 53, 2020",
            "weeklynote",
        ),
        // 2024-12-31 is in week 1 of 2025.
        (
            "week",
            "2024-12-31",
            "2025-01-05",
            "2024-12-30",
            "Week 1, 2025",
            "weeklynote",
        ),
        (
            "month",
            "2026-02-28",
            "2026-02-01",
            "2026-02-01",
            "February 2026",
            "monthlynote",
        ),
        (
            "quarter",
            "2026-12-31",
            "2026-11-15",
            "2026-10-01",
            "Q4 2026",
            "quarterlynote",
        ),
        (
            "year",
            "2026-10-25",
            "2026-01-01",
            "2026-01-01",
            "2026",
            "yearlynote",
        ),
    ];
    for (period, day, other, start, title, ty) in cases {
        expect(
            api.send("GET", &format!("/api/planner/{period}/{day}")),
            StatusCode::NOT_FOUND,
        );
        let made = expect(
            api.send("POST", &format!("/api/planner/{period}/{day}")),
            StatusCode::CREATED,
        );
        assert_eq!(made["created"], true);
        assert_eq!(made["type"], ty);
        assert_eq!(made["note"]["fields"]["title"], title, "{made}");
        assert_eq!(made["note"]["fields"]["date"], start, "{made}");
        let again = expect(
            api.send("POST", &format!("/api/planner/{period}/{other}")),
            StatusCode::OK,
        );
        assert_eq!(again["id"], made["id"]);
        assert_eq!(again["created"], false);
        let read = expect(
            api.send("GET", &format!("/api/planner/{period}/{other}")),
            StatusCode::OK,
        );
        assert_eq!(read["id"], made["id"]);
    }
}

#[test]
fn the_daily_route_and_the_planner_day_are_one_note() {
    let api = Api::new(TestServer::start());
    let made = expect(
        api.send("POST", "/api/planner/day/2026-10-13"),
        StatusCode::CREATED,
    );
    assert_eq!(made["type"], "dailynote");
    assert_eq!(made["note"]["fields"]["title"], "Tuesday 13 October 2026");
    let daily = expect(api.send("GET", "/api/daily/2026-10-13"), StatusCode::OK);
    assert_eq!(daily["id"], made["id"]);
    let opened = expect(
        api.send("POST", "/api/daily/2026-10-14"),
        StatusCode::CREATED,
    );
    let planner = expect(
        api.send("GET", "/api/planner/day/2026-10-14"),
        StatusCode::OK,
    );
    assert_eq!(planner["id"], opened["id"]);
}

#[test]
fn a_period_or_day_the_planner_does_not_know_is_a_400() {
    let api = Api::new(TestServer::start());
    for path in [
        "/api/planner/fortnight/2026-10-13",
        "/api/planner/week/2026-02-30",
        "/api/planner/month/today",
    ] {
        expect(api.send("GET", path), StatusCode::BAD_REQUEST);
        expect(api.send("POST", path), StatusCode::BAD_REQUEST);
    }
}

#[test]
fn an_existing_vault_gains_the_period_note_types_and_keeps_its_daily_notes() {
    // A vault from before the planner: the daily note type and one daily note.
    const DAY: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZA";
    let api = Api::new(TestServer::start_seeded(|vault| {
        std::fs::create_dir_all(vault.join("types")).unwrap();
        std::fs::create_dir_all(vault.join("notes/dailynote")).unwrap();
        std::fs::write(
            vault.join("types/dailynote.wcl"),
            writer::DEFAULT_DAILY_TYPE,
        )
        .unwrap();
        std::fs::write(
            vault.join(format!("notes/dailynote/{DAY}.wcl")),
            "title = \"Tuesday 13 October 2026\"\ncreated = \"2026-10-13T09:00:00Z\"\nupdated = \"2026-10-13T09:00:00Z\"\ndate = \"2026-10-13\"\n",
        )
        .unwrap();
    }));
    let types = expect(api.send("GET", "/api/types"), StatusCode::OK);
    for name in PERIOD_TYPES {
        let ty = types["types"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == name)
            .unwrap_or_else(|| panic!("no {name} in {types}"));
        assert_eq!(ty["builtin"], true, "{ty}");
    }
    assert_eq!(api.type_file("weeklynote"), writer::DEFAULT_WEEKLY_TYPE);
    assert_eq!(api.type_file("dailynote"), writer::DEFAULT_DAILY_TYPE);
    let day = expect(
        api.send("GET", "/api/planner/day/2026-10-13"),
        StatusCode::OK,
    );
    assert_eq!(day["id"], DAY);
    let week = expect(
        api.send("POST", "/api/planner/week/2026-10-13"),
        StatusCode::CREATED,
    );
    assert_eq!(week["note"]["fields"]["date"], "2026-10-12");
}
