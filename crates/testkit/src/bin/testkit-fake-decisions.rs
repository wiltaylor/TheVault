//! Loopback decisions fake. Each stdin line queues a JSON response, or `requests` reads the count.
use std::io::{BufRead, Write};

use serde_json::{Value, json};
use testkit::{FakeDecisions, Reply};

fn main() {
    let fake = FakeDecisions::start();
    println!(
        "{}",
        json!({"base_url": fake.base_url(), "api_key": FakeDecisions::KEY})
    );
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let answer = if line.trim() == "requests" {
            json!({"count": fake.requests().len()})
        } else {
            match serde_json::from_str::<Value>(&line) {
                Ok(value) => {
                    fake.push(Reply::json(200, &value));
                    json!({"ok": "push"})
                }
                Err(_) => json!({"error": "expected JSON response or requests"}),
            }
        };
        println!("{answer}");
        let _ = std::io::stdout().flush();
    }
}
