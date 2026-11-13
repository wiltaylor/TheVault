//! Owner-selected Gmail parts. Reading metadata never contacts the provider.
use super::*;
use axum::extract::Path;
use writer::{CheckedNoteWrite, Input, doc_to_json};

const MAX_BYTES: usize = 10 * 1024 * 1024;
const MAX_RESPONSE: usize = MAX_BYTES.div_ceil(3) * 4 + 4096;
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct Part {
    part: String,
    attachment: String,
    filename: String,
    size: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Selection {
    account: String,
    message: String,
    part: String,
}

fn refused() -> ApiError {
    ApiError::bad_request("Gmail attachment selection is unavailable or changed")
}

fn provider_error() -> ApiError {
    ApiError::bad_request("Gmail attachment fetch failed or exceeded its limits")
}

fn owner(author: &state::Author) -> Result<(), ApiError> {
    if !matches!(author, state::Author::Session(_)) {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "session_required",
            "Sign in to fetch Gmail attachments",
        ));
    }
    Ok(())
}

fn parts(note: &writer::Note) -> Result<Vec<Part>, ApiError> {
    if note.type_name != "email" {
        return Err(refused());
    }
    let note = doc_to_json(&note.doc);
    let rows = note["fields"]["gmail_deferred_parts"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if rows.len() > 100 {
        return Err(refused());
    }
    rows.iter()
        .map(|row| {
            let raw = row
                .as_str()
                .filter(|s| s.len() <= 4096)
                .ok_or_else(refused)?;
            let part: Part = serde_json::from_str(raw).map_err(|_| refused())?;
            if part.part.is_empty()
                || part.part.len() > 256
                || part.attachment.is_empty()
                || part.attachment.len() > 2048
                || part.size <= vocab::GMAIL_IMPORT_ATTACHMENT_BYTES
                || part.filename.is_empty()
                || part.filename.len() > 255
                || part.filename == "."
                || part.filename == ".."
                || part
                    .filename
                    .chars()
                    .any(|c| c == '/' || c == '\\' || c.is_control())
            {
                return Err(refused());
            }
            Ok(part)
        })
        .collect()
}

fn select(w: &Writer, id: &str, selected: &Selection) -> Result<(writer::Note, Part), ApiError> {
    let note = w.read(id).map_err(|_| refused())?;
    let json = doc_to_json(&note.doc);
    let account = w.state().get_secret(ACCOUNT)?.ok_or_else(refused)?;
    if account.is_empty()
        || account != selected.account
        || json["fields"]["gmail_account"] != account
        || json["fields"]["gmail_ref"] != selected.message
        || selected.message.is_empty()
        || selected.message.len() > 256
    {
        return Err(refused());
    }
    let matching: Vec<_> = parts(&note)?
        .into_iter()
        .filter(|p| p.part == selected.part)
        .collect();
    if matching.len() != 1 || matching[0].size > MAX_BYTES {
        return Err(refused());
    }
    Ok((note, matching[0].clone()))
}

pub(crate) async fn preview(
    State(g): Shared,
    crate::Caller(author): crate::Caller,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    owner(&author)?;
    on_writer(&g, move |w| {
        let note = w.read(&id).map_err(|_| refused())?;
        let json = doc_to_json(&note.doc);
        // No credentials or provider attachment identifiers leave this route.
        let rows: Vec<_> = parts(&note)?.iter().map(|p| json!({
            "part":p.part,"filename":p.filename,"size":p.size,"fetchable":p.size <= MAX_BYTES
        })).collect();
        Ok(Json(json!({"account":json["fields"]["gmail_account"],
            "message":json["fields"]["gmail_ref"],"parts":rows,"max_bytes":MAX_BYTES,"timeout_seconds":30})))
    }).await
}

async fn bounded_json(response: reqwest::Response, cap: usize) -> Result<Value, ApiError> {
    if !response.status().is_success() || response.content_length().is_some_and(|n| n > cap as u64)
    {
        return Err(provider_error());
    }
    let mut response = response;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| provider_error())? {
        if chunk.len() > cap.saturating_sub(bytes.len()) {
            return Err(provider_error());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| provider_error())
}

// Encode each identifier as one URL segment, including slashes and dot segments.
fn segment(text: &str) -> String {
    text.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

async fn within_budget<T>(
    work: impl std::future::Future<Output = Result<T, ApiError>>,
) -> Result<T, ApiError> {
    tokio::time::timeout(FETCH_TIMEOUT, work)
        .await
        .map_err(|_| provider_error())?
}

pub(crate) async fn fetch(
    State(g): Shared,
    crate::Caller(author): crate::Caller,
    Path(id): Path<String>,
    Json(selected): Json<Selection>,
) -> Result<Json<Value>, ApiError> {
    owner(&author)?;
    let _permit = g.attachment_fetch.try_acquire().map_err(|_| {
        ApiError::new(
            StatusCode::CONFLICT,
            "busy",
            "A Gmail attachment fetch is already running",
        )
    })?;
    let check_id = id.clone();
    let check = Selection {
        account: selected.account.clone(),
        message: selected.message.clone(),
        part: selected.part.clone(),
    };
    let (note, part, config, credentials) = on_writer(&g, move |w| {
        let (note, part) = select(w, &check_id, &check)?;
        let settings = settings(w)?;
        let secret = |name| {
            w.state()
                .get_secret(name)
                .map_err(ApiError::from)?
                .ok_or_else(refused)
        };
        Ok((
            note,
            part,
            settings,
            [
                secret(CLIENT_ID)?,
                secret(CLIENT_SECRET)?,
                secret(REFRESH_TOKEN)?,
            ],
        ))
    })
    .await?;
    let endpoints = Endpoints::of(&config);
    let gmail = endpoints
        .dict
        .as_ref()
        .and_then(|d| d.get("gmail"))
        .and_then(Value::as_str)
        .unwrap_or("https://gmail.googleapis.com/gmail/v1")
        .trim_end_matches('/');
    // Disable redirects so neither refresh nor access tokens are forwarded elsewhere.
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(FETCH_TIMEOUT)
        .build()
        .map_err(|_| provider_error())?;
    let bytes = within_budget(async {
        let response = http
            .post(&endpoints.token)
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", credentials[0].as_str()),
                ("client_secret", credentials[1].as_str()),
                ("refresh_token", credentials[2].as_str()),
            ])
            .send()
            .await
            .map_err(|_| provider_error())?;
        let token = bounded_json(response, 65536).await?;
        let access = token["access_token"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(provider_error)?;
        let url = format!(
            "{gmail}/users/me/messages/{}/attachments/{}",
            segment(&selected.message),
            segment(&part.attachment)
        );
        let response = http
            .get(url)
            .bearer_auth(access)
            .send()
            .await
            .map_err(|_| provider_error())?;
        let data = bounded_json(response, MAX_RESPONSE).await?;
        let bytes = URL_SAFE_NO_PAD
            .decode(data["data"].as_str().ok_or_else(provider_error)?)
            .map_err(|_| provider_error())?;
        if bytes.len() != part.size || bytes.len() > MAX_BYTES {
            return Err(provider_error());
        }
        Ok(bytes)
    })
    .await?;
    on_writer(&g, move |w| {
        let (current, _) = select(w, &id, &selected)?;
        if current.version != note.version || settings(w)? != config {
            return Err(refused());
        }
        // Hold before asset publication. This also removes the legacy-allowed exception.
        w.state().hold_for_new_attachment(&id)?;
        let asset_id = ulid::Ulid::new().to_string();
        w.mark_incoming_asset(&asset_id)?;
        let asset = w.put_asset_with_id(&asset_id, &part.filename, bytes)?;
        let mut next = doc_to_json(&note.doc);
        let mut attachments = next["fields"]["attachments"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        attachments.push(json!(asset.id));
        next["fields"]["attachments"] = json!(attachments);
        let remaining: Vec<_> = parts(&note)?
            .into_iter()
            .filter(|p| p.part != selected.part)
            .map(|p| serde_json::to_string(&p).expect("part"))
            .collect();
        next["fields"]["gmail_deferred_parts"] = json!(remaining);
        let input = Input::Json(next);
        w.commit_checked_note(
            CheckedNoteWrite::Update(&id, &input, Some(&note.version)),
            &author,
            |w| {
                select(w, &id, &selected)?;
                if settings(w)? != config {
                    return Err(refused());
                }
                Ok::<_, ApiError>(())
            },
        )?;
        Ok(Json(
            json!({"asset":asset.id,"agent_safety":w.agent_safety(&id)?}),
        ))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tower::ServiceExt;

    // Exercise the route with real HTTP framing from a local synthetic provider.
    async fn route_failure(mode: &'static str) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (arrived, received) = tokio::sync::oneshot::channel();
        let provider = tokio::spawn(async move {
            for step in 0..2 {
                let (mut conn, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    let mut buf = [0; 4096];
                    let n = conn.read(&mut buf).await.unwrap();
                    assert_ne!(n, 0);
                    request.extend_from_slice(&buf[..n]);
                }
                if step == 0 {
                    let body = r#"{"access_token":"fake"}"#;
                    conn.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
                    continue;
                }
                let _ = arrived.send(());
                match mode {
                    "length" => {
                        // The advertised length is false. No large body is needed to refuse it.
                        let _ = conn.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{{}}", MAX_RESPONSE + 1).as_bytes()).await;
                    }
                    "body" => {
                        let _ = conn.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").await;
                        let chunk = vec![b'x'; MAX_RESPONSE + 1];
                        let _ = conn.write_all(format!("{:x}\r\n", chunk.len()).as_bytes()).await;
                        let _ = conn.write_all(&chunk).await;
                        let _ = conn.write_all(b"\r\n0\r\n\r\n").await;
                    }
                    "size" => {
                        let body = r#"{"data":"YWJj"}"#;
                        let _ = conn.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await;
                    }
                    "stall" => std::future::pending::<()>().await,
                    _ => unreachable!(),
                }
                break;
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_owned();
        let (google, id) = tokio::task::spawn_blocking(move || {
            let vault = store::Vault::init(&path).unwrap();
            let state = state::State::open(&vault.server_dir().join("state.db"), Some(state::SecretKey::generate())).unwrap();
            let w = Arc::new(Writer::open(vault, state).unwrap());
            w.upgrade_shipped_defaults(writer::Upkeep::Home).unwrap();
            for name in [ACCOUNT, CLIENT_ID, CLIENT_SECRET, REFRESH_TOKEN] {
                w.state().set_secret(name, "fake").unwrap();
            }
            w.state().set_secret(SETTINGS, &json!({"endpoints":{"token":format!("http://{addr}/token"),"gmail":format!("http://{addr}")}}).to_string()).unwrap();
            let row = json!({"part":"part","attachment":"attachment","filename":"invoice.csv","size":800000}).to_string();
            let note = w.create("email", &Input::Json(json!({"fields":{"title":"Mail","gmail_ref":"message","gmail_account":"fake","gmail_deferred_parts":[row]}})), &state::Author::Task("google_gmail".into())).unwrap();
            let host = Arc::new(script::Host::new(w.clone(), 1, script::Limits::default(), script::Ai::of(&w)));
            let scheduler = Scheduler::new(w, host, Arc::new(scheduler::TestClock::new(0))).unwrap();
            (Arc::new(Google::new(scheduler)), note.id)
        }).await.unwrap();
        let app = axum::Router::new().route("/{id}", axum::routing::post(fetch))
            .layer(axum::Extension(crate::Caller(state::Author::Session("owner".into()))))
            .with_state(google.clone());
        let request = axum::http::Request::builder().method("POST").uri(format!("/{id}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(json!({"account":"fake","message":"message","part":"part"}).to_string())).unwrap();
        let route = tokio::spawn(app.oneshot(request));
        received.await.unwrap();
        if mode == "stall" {
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(29)).await;
            assert!(!route.is_finished());
            tokio::time::advance(Duration::from_secs(1)).await;
        }
        let response = route.await.unwrap().unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("Gmail attachment fetch failed or exceeded its limits"));
        provider.abort();
        if mode == "stall" { tokio::time::resume(); }
        tokio::task::spawn_blocking(move || {
            let note = google.writer().read(&id).unwrap();
            let json = doc_to_json(&note.doc);
            assert!(json["fields"]["attachments"].is_null());
            assert_eq!(json["fields"]["gmail_deferred_parts"].as_array().unwrap().len(), 1);
        }).await.unwrap();
    }

    #[tokio::test]
    async fn route_refuses_false_content_length() { route_failure("length").await; }
    #[tokio::test]
    async fn route_refuses_oversized_stream() { route_failure("body").await; }
    #[tokio::test]
    async fn route_refuses_decoded_size_mismatch() { route_failure("size").await; }
    #[tokio::test]
    async fn route_stalled_provider_uses_thirty_second_budget() { route_failure("stall").await; }

    #[tokio::test(start_paused = true)]
    async fn stalled_fetch_has_one_total_deadline_with_fake_clock() {
        let start = tokio::time::Instant::now();
        let result = within_budget(std::future::pending::<Result<(), ApiError>>()).await;
        assert!(result.is_err());
        assert_eq!(start.elapsed(), FETCH_TIMEOUT);
    }

    #[tokio::test]
    async fn response_bytes_and_json_are_bounded() {
        let response = reqwest::Response::from(axum::http::Response::new("{\"data\":\"AAAA\"}"));
        assert!(bounded_json(response, 4).await.is_err());
        let response =
            reqwest::Response::from(axum::http::Response::new("not JSON: provider secret"));
        assert!(bounded_json(response, 100).await.is_err());
        let response = reqwest::Response::from(
            axum::http::Response::builder()
                .status(500)
                .body("provider secret")
                .unwrap(),
        );
        assert!(bounded_json(response, 100).await.is_err());
        let response = reqwest::Response::from(axum::http::Response::new("{\"data\":\"AAAA\"}"));
        assert_eq!(bounded_json(response, 100).await.unwrap()["data"], "AAAA");
    }

    #[test]
    fn identifiers_are_single_segments() {
        assert_eq!(segment("a/b?x#z"), "a%2Fb%3Fx%23z");
    }
}
