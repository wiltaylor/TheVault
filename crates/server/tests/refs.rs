//! S1 with S3 checks: two-way and peer references and the `on_delete` rules (D4, D5, D12), seen
//! through the API and in the note files the writes leave behind.

use std::path::Path;

use reqwest::blocking::{Client, Response};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue, IF_MATCH};
use serde_json::{Value, json};
use testkit::TestServer;

const TEMPLATE: &str = r#"
template default {
  tab "Main" {
    slot "title"
  }
}
"#;

/// A project holds its tasks (two-way with `task.project`) and its related projects (peer).
const PROJECT: &str = r#"label = "Project"

field tasks {
  kind     = "reference"
  target   = "task"
  relation = "two_way"
  inverse  = "project"
  multiple = true
}

field related {
  kind     = "reference"
  target   = "project"
  relation = "peer"
  multiple = true
}
"#;

/// A task belongs to one project, which cannot be deleted while the task points at it.
const TASK: &str = r#"label = "Task"

field project {
  kind      = "reference"
  target    = "project"
  relation  = "two_way"
  inverse   = "tasks"
  on_delete = "restrict"
}
"#;

/// A comment goes when its task goes.
const COMMENT: &str = r#"label = "Comment"

field task {
  kind      = "reference"
  target    = "task"
  on_delete = "cascade"
}
"#;

const PERSON: &str = "label = \"Person\"\n";

/// A meeting loses an attendee who is deleted.
const MEETING: &str = r#"label = "Meeting"

field attendees {
  kind     = "reference"
  target   = "person"
  multiple = true
}
"#;

const WCL: &str = "text/plain; charset=utf-8";

fn seed(vault: &Path) {
    let types = vault.join("types");
    std::fs::create_dir_all(&types).unwrap();
    for (name, body) in [
        ("project", PROJECT),
        ("task", TASK),
        ("comment", COMMENT),
        ("person", PERSON),
        ("meeting", MEETING),
    ] {
        std::fs::write(
            types.join(format!("{name}.wcl")),
            format!("{body}{TEMPLATE}"),
        )
        .unwrap();
    }
}

struct Api {
    server: TestServer,
    client: Client,
}

impl Api {
    fn start() -> Api {
        let server = TestServer::start_seeded(seed);
        let mut auth = HeaderMap::new();
        auth.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", server.admin_token())).unwrap(),
        );
        let client = Client::builder().default_headers(auth).build().unwrap();
        Api { server, client }
    }

    /// Create a note of `type_name` from WCL text and return its id.
    fn create(&self, type_name: &str, wcl: &str) -> String {
        s(&self.create_full(type_name, wcl)["id"]).to_owned()
    }

    fn create_full(&self, type_name: &str, wcl: &str) -> Value {
        let res = self
            .client
            .post(self.server.url(&format!("/api/notes?type={type_name}")))
            .header(CONTENT_TYPE, WCL)
            .body(wcl.to_owned())
            .send()
            .unwrap();
        expect(res, 201)
    }

    /// Replace note `id` with WCL text, based on its current version.
    fn update(&self, id: &str, wcl: &str) -> Value {
        let base = self.version(id);
        expect(self.update_at(id, &base, wcl), 200)
    }

    fn update_at(&self, id: &str, base: &str, wcl: &str) -> Response {
        self.client
            .put(self.server.url(&format!("/api/notes/{id}")))
            .header(CONTENT_TYPE, WCL)
            .header(IF_MATCH, format!("\"{base}\""))
            .body(wcl.to_owned())
            .send()
            .unwrap()
    }

    fn delete(&self, id: &str) -> Response {
        let base = self.version(id);
        self.client
            .delete(self.server.url(&format!("/api/notes/{id}")))
            .header(IF_MATCH, format!("\"{base}\""))
            .send()
            .unwrap()
    }

    fn read(&self, id: &str) -> Value {
        let res = self
            .client
            .get(self.server.url(&format!("/api/notes/{id}")))
            .send()
            .unwrap();
        expect(res, 200)
    }

    fn version(&self, id: &str) -> String {
        s(&self.read(id)["version"]).to_owned()
    }

    fn trash_ids(&self) -> Vec<String> {
        let res = self
            .client
            .get(self.server.url("/api/trash"))
            .send()
            .unwrap();
        expect(res, 200)["notes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| s(&n["id"]).to_owned())
            .collect()
    }

    /// The note file on disk, or `None` when there is none.
    fn file(&self, type_name: &str, id: &str) -> Option<String> {
        let path = self
            .server
            .vault_path()
            .join(format!("notes/{type_name}/{id}.wcl"));
        std::fs::read_to_string(path).ok()
    }

    /// The reference ids a note file holds in `field`, read from the file on disk.
    fn refs(&self, type_name: &str, id: &str, field: &str) -> Vec<String> {
        let text = self
            .file(type_name, id)
            .unwrap_or_else(|| panic!("notes/{type_name}/{id}.wcl does not exist"));
        let line = text.lines().find(|l| {
            l.split_once('=')
                .is_some_and(|(name, _)| name.trim() == field)
        });
        let Some(line) = line else {
            return Vec::new();
        };
        let value = line.split_once('=').unwrap().1;
        value
            .split('"')
            .skip(1)
            .step_by(2)
            .map(str::to_owned)
            .collect()
    }
}

/// Assert the status and return the JSON body.
fn expect(res: Response, status: u16) -> Value {
    let got = res.status().as_u16();
    let text = res.text().unwrap();
    assert_eq!(got, status, "unexpected status, body: {text}");
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}: body is not JSON: {text}"))
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_else(|| panic!("{v} is not a string"))
}

fn sorted(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids
}

/// The ids in a response's `also` list: the other notes the write changed.
fn also_ids(res: &Value) -> Vec<String> {
    sorted(
        res["also"]
            .as_array()
            .unwrap_or_else(|| panic!("no `also` in {res}"))
            .iter()
            .map(|n| s(&n["id"]).to_owned())
            .collect(),
    )
}

#[test]
fn linking_a_task_writes_both_files_and_unlinking_clears_both() {
    let api = Api::start();
    let project = api.create("project", "title = \"Importer\"\n");
    let project_v0 = api.version(&project);

    // Link from the single side: the project's list gains the task in the same write.
    let created = api.create_full(
        "task",
        &format!("title = \"Parse\"\nproject = \"{project}\"\n"),
    );
    let task = s(&created["id"]).to_owned();
    assert_eq!(api.refs("task", &task, "project"), [project.as_str()]);
    assert_eq!(api.refs("project", &project, "tasks"), [task.as_str()]);
    assert_eq!(also_ids(&created), [project.as_str()]);
    let also = &created["also"][0];
    assert_eq!(also["type"], "project");
    assert_eq!(also["event"], "update");
    assert_eq!(s(&also["version"]), api.version(&project));
    assert_ne!(s(&also["version"]), project_v0);

    // Link from the list side: the second task's single field is set.
    let other = api.create("task", "title = \"Write\"\n");
    let res = api.update(
        &project,
        &format!("title = \"Importer\"\ntasks = [\"{task}\", \"{other}\"]\n"),
    );
    assert_eq!(also_ids(&res), [other.as_str()]);
    assert_eq!(api.refs("task", &other, "project"), [project.as_str()]);

    // Remove the link from the task: it leaves both files.
    let res = api.update(&task, "title = \"Parse\"\n");
    assert_eq!(also_ids(&res), [project.as_str()]);
    assert!(api.refs("task", &task, "project").is_empty());
    assert_eq!(api.refs("project", &project, "tasks"), [other.as_str()]);

    // Remove the link from the project: the task's field goes too.
    let res = api.update(&project, "title = \"Importer\"\n");
    assert_eq!(also_ids(&res), [other.as_str()]);
    assert!(api.refs("task", &other, "project").is_empty());
    let text = api.file("project", &project).unwrap();
    assert!(!text.contains("tasks"), "{text}");
}

#[test]
fn moving_a_task_updates_the_old_and_the_new_project() {
    let api = Api::start();
    let a = api.create("project", "title = \"A\"\n");
    let b = api.create("project", "title = \"B\"\n");
    let task = api.create("task", &format!("title = \"T\"\nproject = \"{a}\"\n"));
    assert_eq!(api.refs("project", &a, "tasks"), [task.as_str()]);

    let res = api.update(&task, &format!("title = \"T\"\nproject = \"{b}\"\n"));
    assert_eq!(also_ids(&res), sorted(vec![a.clone(), b.clone()]));
    assert!(api.refs("project", &a, "tasks").is_empty());
    assert_eq!(api.refs("project", &b, "tasks"), [task.as_str()]);

    // Taking the task into A's list from A's side takes it out of B's.
    api.update(&a, &format!("title = \"A\"\ntasks = [\"{task}\"]\n"));
    assert_eq!(api.refs("task", &task, "project"), [a.as_str()]);
    assert!(api.refs("project", &b, "tasks").is_empty());
}

#[test]
fn a_peer_add_appears_on_every_member() {
    let api = Api::start();
    let p1 = api.create("project", "title = \"One\"\n");
    let p2 = api.create("project", "title = \"Two\"\n");
    let p3 = api.create("project", "title = \"Three\"\n");
    let p4 = api.create("project", "title = \"Four\"\n");

    // A pair.
    let res = api.update(&p1, &format!("title = \"One\"\nrelated = [\"{p2}\"]\n"));
    assert_eq!(also_ids(&res), [p2.as_str()]);
    assert_eq!(api.refs("project", &p2, "related"), [p1.as_str()]);

    // A third member joins through one member and every member lists the others.
    let res = api.update(&p3, &format!("title = \"Three\"\nrelated = [\"{p1}\"]\n"));
    assert_eq!(also_ids(&res), sorted(vec![p1.clone(), p2.clone()]));
    for (id, others) in [
        (&p1, vec![p2.clone(), p3.clone()]),
        (&p2, vec![p1.clone(), p3.clone()]),
        (&p3, vec![p1.clone(), p2.clone()]),
    ] {
        assert_eq!(sorted(api.refs("project", id, "related")), sorted(others));
    }

    // Adding a fourth through another member reaches all of them.
    api.update(
        &p2,
        &format!("title = \"Two\"\nrelated = [\"{p1}\", \"{p3}\", \"{p4}\"]\n"),
    );
    let all = [&p1, &p2, &p3, &p4];
    for id in all {
        let others: Vec<String> = all
            .iter()
            .filter(|o| **o != id)
            .map(|o| o.to_string())
            .collect();
        assert_eq!(
            sorted(api.refs("project", id, "related")),
            sorted(others),
            "{id}"
        );
    }

    // Taking P4 out of one member's list takes it out of the group.
    api.update(
        &p1,
        &format!("title = \"One\"\nrelated = [\"{p2}\", \"{p3}\"]\n"),
    );
    assert!(api.refs("project", &p4, "related").is_empty());
    assert_eq!(
        sorted(api.refs("project", &p2, "related")),
        sorted(vec![p1.clone(), p3.clone()])
    );
    assert_eq!(
        sorted(api.refs("project", &p3, "related")),
        sorted(vec![p1.clone(), p2.clone()])
    );
}

#[test]
fn writing_the_other_note_with_its_pre_link_version_is_a_conflict() {
    let api = Api::start();
    let project = api.create("project", "title = \"Importer\"\n");
    let before = api.version(&project);

    api.create(
        "task",
        &format!("title = \"Parse\"\nproject = \"{project}\"\n"),
    );
    let after = api.version(&project);
    assert_ne!(before, after);

    let res = expect(
        api.update_at(&project, &before, "title = \"Renamed\"\n"),
        409,
    );
    assert_eq!(res["error"], "conflict");
    assert_eq!(res["current_version"], after.as_str());
    let text = api.file("project", &project).unwrap();
    assert!(text.contains("\"Importer\""), "{text}");

    // The history shows the reverse-side update as its own version.
    let res = api
        .client
        .get(api.server.url(&format!("/api/notes/{project}/history")))
        .send()
        .unwrap();
    assert_eq!(expect(res, 200)["versions"].as_array().unwrap().len(), 2);
}

#[test]
fn deleting_a_target_with_unlink_removes_it_from_referrers() {
    let api = Api::start();
    let ann = api.create("person", "title = \"Ann\"\n");
    let bob = api.create("person", "title = \"Bob\"\n");
    let meeting = api.create(
        "meeting",
        &format!("title = \"Standup\"\nattendees = [\"{ann}\", \"{bob}\"]\n"),
    );
    let meeting_v = api.version(&meeting);

    let res = expect(api.delete(&ann), 200);
    assert_eq!(res["event"], "delete");
    assert_eq!(also_ids(&res), [meeting.as_str()]);
    assert_eq!(res["also"][0]["event"], "update");
    assert!(api.file("person", &ann).is_none());
    assert_eq!(api.refs("meeting", &meeting, "attendees"), [bob.as_str()]);
    assert_ne!(api.version(&meeting), meeting_v);
    assert_eq!(api.trash_ids(), [ann.as_str()]);

    // The last reference goes and so does the field.
    expect(api.delete(&bob), 200);
    let text = api.file("meeting", &meeting).unwrap();
    assert!(!text.contains("attendees"), "{text}");
}

#[test]
fn deleting_a_target_with_restrict_is_refused_and_lists_the_referrers() {
    let api = Api::start();
    let project = api.create("project", "title = \"Importer\"\n");
    let t1 = api.create(
        "task",
        &format!("title = \"One\"\nproject = \"{project}\"\n"),
    );
    let t2 = api.create(
        "task",
        &format!("title = \"Two\"\nproject = \"{project}\"\n"),
    );
    let files_before = [
        api.file("project", &project).unwrap(),
        api.file("task", &t1).unwrap(),
        api.file("task", &t2).unwrap(),
    ];
    let versions_before = [api.version(&project), api.version(&t1), api.version(&t2)];

    let res = expect(api.delete(&project), 409);
    assert_eq!(res["error"], "restricted");
    assert_eq!(res["id"], project.as_str());
    let mut referrers: Vec<Value> = res["referrers"].as_array().unwrap().clone();
    referrers.sort_by_key(|r| r["id"].as_str().unwrap().to_owned());
    let mut want = vec![
        json!({ "id": t1, "type": "task", "field": "project" }),
        json!({ "id": t2, "type": "task", "field": "project" }),
    ];
    want.sort_by_key(|r| r["id"].as_str().unwrap().to_owned());
    assert_eq!(referrers, want);
    let message = s(&res["message"]);
    assert!(message.contains(&t1) && message.contains(&t2), "{message}");

    // Nothing changed.
    let files_after = [
        api.file("project", &project).unwrap(),
        api.file("task", &t1).unwrap(),
        api.file("task", &t2).unwrap(),
    ];
    assert_eq!(files_before, files_after);
    let versions_after = [api.version(&project), api.version(&t1), api.version(&t2)];
    assert_eq!(versions_before, versions_after);
    assert!(api.trash_ids().is_empty());
}

#[test]
fn deleting_a_target_with_cascade_trashes_the_referrers() {
    let api = Api::start();
    let project = api.create("project", "title = \"Importer\"\n");
    let task = api.create(
        "task",
        &format!("title = \"Parse\"\nproject = \"{project}\"\n"),
    );
    let c1 = api.create(
        "comment",
        &format!("title = \"Looks good\"\ntask = \"{task}\"\n"),
    );
    let c2 = api.create(
        "comment",
        &format!("title = \"Ship it\"\ntask = \"{task}\"\n"),
    );
    let other = api.create("task", "title = \"Other\"\n");
    let kept = api.create(
        "comment",
        &format!("title = \"Elsewhere\"\ntask = \"{other}\"\n"),
    );

    let res = expect(api.delete(&task), 200);
    // The comments go, and the project's two-way list loses the task (its field unlinks).
    assert_eq!(
        also_ids(&res),
        sorted(vec![c1.clone(), c2.clone(), project.clone()])
    );
    for c in [&c1, &c2] {
        assert!(
            api.file("comment", c).is_none(),
            "comment {c} is still there"
        );
    }
    assert!(api.file("comment", &kept).is_some());
    assert!(api.refs("project", &project, "tasks").is_empty());
    assert_eq!(
        sorted(api.trash_ids()),
        sorted(vec![task.clone(), c1.clone(), c2.clone()])
    );

    // A cascade stops at a restrict: deleting the project is fine now, but a project with a task
    // that has comments is refused as a whole.
    let t2 = api.create(
        "task",
        &format!("title = \"Again\"\nproject = \"{project}\"\n"),
    );
    api.create("comment", &format!("title = \"Hm\"\ntask = \"{t2}\"\n"));
    let res = expect(api.delete(&project), 409);
    assert_eq!(res["referrers"][0]["id"], t2.as_str());
    assert!(api.file("task", &t2).is_some());
}
