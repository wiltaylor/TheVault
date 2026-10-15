//! The home server's HTTP API, as the device's sync engine calls it with its device token.

use reqwest::StatusCode;
use reqwest::blocking::{Client, RequestBuilder};
use reqwest::header::{ACCEPT, AUTHORIZATION, IF_MATCH, IF_NONE_MATCH};
use serde_json::Value;

use crate::{Error, Remote, Result};

/// An answer from the server: its status and its body as JSON (`Null` when it is not JSON).
pub(crate) struct Answer {
    pub status: StatusCode,
    pub body: Value,
}

impl Answer {
    /// The body of a success, or the error the status and body describe.
    pub fn ok(self, what: &str) -> Result<Value> {
        if self.status.is_success() {
            Ok(self.body)
        } else {
            Err(Error::Server {
                what: what.to_owned(),
                status: self.status.as_u16(),
                body: self.body.to_string(),
            })
        }
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        self.body.get(key).and_then(Value::as_str)
    }
}

/// The server of one paired device.
pub(crate) struct Home {
    client: Client,
    remote: Remote,
}

impl Home {
    pub fn new(remote: Remote) -> Result<Home> {
        let client = Client::builder()
            .build()
            .map_err(|e| Error::Http(e.to_string()))?;
        Ok(Home { client, remote })
    }

    pub fn remote(&self) -> &Remote {
        &self.remote
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.remote.url.trim_end_matches('/'))
    }

    fn authed(&self, rb: RequestBuilder) -> RequestBuilder {
        rb.header(AUTHORIZATION, format!("Bearer {}", self.remote.token))
            .header(ACCEPT, "application/json")
    }

    fn send(&self, rb: RequestBuilder) -> Result<Answer> {
        let res = rb.send().map_err(|e| Error::Http(e.to_string()))?;
        let status = res.status();
        let text = res.text().map_err(|e| Error::Http(e.to_string()))?;
        let body = serde_json::from_str(&text).unwrap_or(Value::Null);
        Ok(Answer { status, body })
    }

    pub fn get(&self, path: &str) -> Result<Answer> {
        self.send(self.authed(self.client.get(self.url(path))))
    }

    /// `GET /api/sync/info`, which needs no token.
    pub fn info(&self) -> Result<Answer> {
        self.send(self.client.get(self.url("/api/sync/info")))
    }

    pub fn post_json(&self, path: &str, body: &Value) -> Result<Answer> {
        self.send(self.authed(self.client.post(self.url(path))).json(body))
    }

    pub fn put_json(&self, path: &str, body: &Value) -> Result<Answer> {
        self.send(self.authed(self.client.put(self.url(path))).json(body))
    }

    /// `POST` with no body and the note's base in `If-Match`.
    pub fn post_at(&self, path: &str, base: &str) -> Result<Answer> {
        self.send(
            self.authed(self.client.post(self.url(path)))
                .header(IF_MATCH, format!("\"{base}\"")),
        )
    }

    /// `PUT /api/notes/{id}` with `body` (`{"wcl"}` or `{"note"}`), at `base`, or creating the
    /// note of `type_name` under its id when `base` is `None`.
    pub fn put_note(
        &self,
        id: &str,
        type_name: &str,
        body: &Value,
        base: Option<&str>,
    ) -> Result<Answer> {
        let rb = self
            .authed(
                self.client
                    .put(self.url(&format!("/api/notes/{id}?type={type_name}"))),
            )
            .json(body);
        let rb = match base {
            Some(base) => rb.header(IF_MATCH, format!("\"{base}\"")),
            None => rb.header(IF_NONE_MATCH, "*"),
        };
        self.send(rb)
    }

    pub fn delete_note(&self, id: &str, base: &str) -> Result<Answer> {
        self.send(
            self.authed(self.client.delete(self.url(&format!("/api/notes/{id}"))))
                .header(IF_MATCH, format!("\"{base}\"")),
        )
    }

    pub fn put_asset(&self, id: &str, filename: &str, bytes: Vec<u8>) -> Result<Answer> {
        self.send(
            self.authed(self.client.put(self.url(&format!("/api/assets/{id}"))))
                .query(&[("filename", filename)])
                .body(bytes),
        )
    }

    /// Asset `id`'s bytes.
    pub fn asset(&self, id: &str) -> Result<Vec<u8>> {
        let res = self
            .client
            .get(self.url(&format!("/api/assets/{id}")))
            .header(AUTHORIZATION, format!("Bearer {}", self.remote.token))
            .send()
            .map_err(|e| Error::Http(e.to_string()))?;
        let status = res.status();
        if !status.is_success() {
            return Err(Error::Server {
                what: format!("fetching asset {id}"),
                status: status.as_u16(),
                body: res.text().unwrap_or_default(),
            });
        }
        Ok(res
            .bytes()
            .map_err(|e| Error::Http(e.to_string()))?
            .to_vec())
    }
}
