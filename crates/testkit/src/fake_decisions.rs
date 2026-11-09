//! The documented OpenRouter alpha decisions contract, not the chat API.
//! Fixtures deliberately do not import the AI client's wire types.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::fake_http::{Answer, Listener, listen, lock};
use crate::{Reply, Request};

pub struct FakeDecisions {
    listener: Listener,
    requests: Arc<Mutex<Vec<Request>>>,
    replies: Arc<Mutex<VecDeque<Answer>>>,
}

impl Default for FakeDecisions {
    fn default() -> Self {
        Self::start()
    }
}

impl FakeDecisions {
    pub const KEY: &str = "test-decisions-key";

    pub fn start() -> Self {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let replies = Arc::new(Mutex::new(VecDeque::new()));
        let seen = requests.clone();
        let queued = replies.clone();
        let listener = listen("decisions", move |request| {
            lock(&seen).push(request.clone());
            let error = |status, message| {
                Answer::Whole(Reply::json(status, &json!({"error": {"message": message}})))
            };
            if request.path != "/api/alpha/decisions" || request.method != "POST" {
                return error(404, "Wrong decisions endpoint");
            }
            if request.header("authorization") != Some("Bearer test-decisions-key") {
                return error(401, "Invalid test key");
            }
            let Ok(body) = serde_json::from_slice::<Value>(&request.body) else {
                return error(400, "Invalid JSON");
            };
            if body.get("tools").is_some()
                || body.get("stream").is_some()
                || body["model"].as_str().is_none_or(str::is_empty)
                || !(body["state"].is_string()
                    || body["state"].is_object()
                    || body["state"].is_array())
                || body["questions"].as_object().is_none_or(|q| {
                    q.is_empty()
                        || q.values().any(|v| {
                            !matches!(v["type"].as_str(), Some("noul" | "choice" | "score"))
                                || v.get("instructions").is_none()
                        })
                })
            {
                return error(400, "Invalid decisions request");
            }
            lock(&queued)
                .pop_front()
                .unwrap_or_else(|| error(500, "No queued decision"))
        });
        Self {
            listener,
            requests,
            replies,
        }
    }

    pub fn base_url(&self) -> String {
        format!("http://{}/api/v1", self.listener.addr)
    }
    pub fn requests(&self) -> Vec<Request> {
        lock(&self.requests).clone()
    }
    pub fn push(&self, reply: Reply) {
        lock(&self.replies).push_back(Answer::Whole(reply));
    }
    pub fn push_chunked(&self, body: &Value) {
        lock(&self.replies).push_back(Answer::JsonChunks(Reply::json(200, body)));
    }
}
