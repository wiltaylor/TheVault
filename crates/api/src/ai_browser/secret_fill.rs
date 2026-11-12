//! Server-owned confirmation and fill authorization. No model confirmation flag.
use super::*;

pub(crate) async fn call(
    b: &BrowserBox,
    state: &state::State,
    agent: &schema::Agent,
    call_id: &str,
    input: &Value,
    unattended: bool,
    write: bool,
) -> BrowserOut {
    if !write {
        return outcome(state, agent, input, "", "insufficient_scope");
    }
    if !agent.permissions.confirm.secrets {
        return authorized(b, state, agent, input).await;
    }
    if unattended {
        return outcome(state, agent, input, "", "confirmation_required");
    }
    match proposal(b, state, agent, call_id, input).await {
        Ok(p) => {
            let mut out = BrowserOut::done(
                json!({"proposed":true,"status":"confirmation_required"}).to_string(),
                json!({"notes":[],"message":"Awaiting owner Apply"}),
            );
            out.event = Some(("proposal", p));
            out
        }
        Err(reason) => outcome(state, agent, input, "", reason),
    }
}

async fn proposal(
    b: &BrowserBox,
    state: &state::State,
    agent: &schema::Agent,
    call_id: &str,
    input: &Value,
) -> Result<Value, &'static str> {
    let name = input["secret"].as_str().ok_or("invalid_request")?;
    if !agent.permissions.browser
        || !agent.permissions.browser_secrets.iter().any(|s| s == name)
        || !state
            .browser_secret_active(&agent.name, name)
            .map_err(|_| "secret_unavailable")?
    {
        return Err("secret_not_allowed");
    }
    let field = input["field"].as_str().ok_or("invalid_request")?;
    if !matches!(field, "username" | "password" | "otp") {
        return Err("invalid_field");
    }
    let target = input["target"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 1024)
        .ok_or("invalid_request")?;
    let session = input["session"]
        .as_str()
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 80
                && s.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        })
        .ok_or("invalid_request")?;
    let page = request(
        b,
        "/secret/origin",
        json!({"session":session,"target":target}),
    )
    .await?;
    let origin = state::canonical_origin(page["origin"].as_str().ok_or("invalid_origin")?)
        .ok_or("invalid_origin")?;
    let login = state
        .get_login_secret(name)
        .map_err(|_| "secret_unavailable")?
        .ok_or("unknown_secret")?;
    if !login.origins.is_empty() && !login.origins.contains(&origin) {
        return Err("origin_mismatch");
    }
    let p = json!({"id":call_id,"agent":agent.name,"op":"browser_secret","secret":name,"field":field,"target":target,"session":session,"origin":origin,"browser_profile":b.profile,"unrestricted_origin":login.origins.is_empty()});
    let redactor =
        crate::browser_redaction::Redactor::load(state).map_err(|_| "secret_unavailable")?;
    // Do not publish a credential even when it coincides with request metadata. Do not
    // silently change the selector/origin the owner is being asked to authorize either.
    if p.as_object()
        .expect("object")
        .values()
        .filter_map(Value::as_str)
        .any(|s| redactor.text(s) != s)
    {
        return Err("unsafe_metadata");
    }
    Ok(p)
}

/// Called only after the owner guard on POST /api/ai/apply, from its blocking task.
pub(crate) fn apply(w: &Writer, author: &Author, p: &Value) -> Result<Value, ApplyError> {
    let name = match author {
        Author::Agent(name) => name,
        _ => return Err(ApplyError::Refused("invalid_agent".into())),
    };
    let agent = w.agent(name)?;
    let b = BrowserBox::from_env()
        .ok_or_else(|| ApplyError::Refused("browser_unavailable".into()))?
        .with_profile(w.browser_profile(name)?);
    if p["browser_profile"].as_str() != b.profile.as_deref() || p["origin"].as_str().is_none() {
        return Err(ApplyError::Refused("stale_browser_proposal".into()));
    }
    let handle = tokio::runtime::Handle::try_current()
        .map_err(|_| ApplyError::Refused("browser_unavailable".into()))?;
    let out = handle.block_on(authorized(&b, w.state(), &agent, p));
    if let Some(trail) = out.trail {
        eprintln!("{trail}");
    }
    if out.is_error {
        return Err(ApplyError::Refused(
            out.summary["message"]
                .as_str()
                .unwrap_or("fill_refused")
                .into(),
        ));
    }
    Ok(json!({"op":"browser_secret","status":"filled"}))
}

fn outcome(
    state: &state::State,
    agent: &schema::Agent,
    input: &Value,
    origin: &str,
    status: &str,
) -> BrowserOut {
    let mut out = if status == "filled" {
        BrowserOut::done("filled".into(), json!({"status":"filled"}))
    } else {
        BrowserOut::error(status)
    };
    let audit = json!({"agent": agent.name, "secret": input["secret"].as_str().unwrap_or(""),
        "field": input["field"].as_str().unwrap_or(""), "origin":origin,
        "result": if status == "filled" { "allowed" } else { "refused" }});
    // Redact decoded strings before JSON escaping. Even metadata can equal a credential.
    let safe = match crate::browser_redaction::Redactor::load(state) {
        Ok(r) => {
            let mut audit = audit;
            for value in audit.as_object_mut().expect("object").values_mut() {
                *value = json!(r.text(value.as_str().unwrap_or("")));
            }
            audit.to_string()
        },
        Err(_) => json!({"agent":"withheld","secret":"withheld","field":"withheld","origin":"withheld","result":"refused"}).to_string(),
    };
    out.trail = Some(format!("browser_secret: {safe}"));
    out
}

pub(crate) async fn authorized(
    b: &BrowserBox,
    state: &state::State,
    agent: &schema::Agent,
    input: &Value,
) -> BrowserOut {
    let mut origin = String::new();
    let status = fill(b, state, agent, input, &mut origin)
        .await
        .unwrap_or_else(|reason| reason);
    outcome(state, agent, input, &origin, status)
}

async fn request(b: &BrowserBox, path: &str, body: Value) -> Result<Value, &'static str> {
    let url = b.url(path, &[]).map_err(|_| "browser_unavailable")?;
    let mut response = b
        .auth(http().post(url))
        .json(&body)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|_| "browser_unavailable")?;
    if !response.status().is_success() {
        return Err("browser_refused");
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "browser_unavailable")? {
        if bytes.len() + chunk.len() > 4096 {
            return Err("browser_refused");
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "browser_refused")
}

async fn fill(
    b: &BrowserBox,
    state: &state::State,
    agent: &schema::Agent,
    input: &Value,
    origin: &mut String,
) -> Result<&'static str, &'static str> {
    let name = input["secret"].as_str().ok_or("invalid_request")?;
    let field = input["field"].as_str().ok_or("invalid_request")?;
    let target = input["target"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 1024)
        .ok_or("invalid_request")?;
    let session = input["session"]
        .as_str()
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 80
                && s.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        })
        .ok_or("invalid_request")?;
    if !matches!(field, "username" | "password" | "otp") {
        return Err("invalid_field");
    }
    if !agent.permissions.browser || !agent.permissions.browser_secrets.iter().any(|s| s == name) {
        return Err("secret_not_allowed");
    }
    if !state
        .is_login_secret(name)
        .map_err(|_| "secret_unavailable")?
    {
        return Err("unknown_secret");
    }
    if !state
        .browser_secret_active(&agent.name, name)
        .map_err(|_| "secret_unavailable")?
    {
        return Err("secret_not_allowed");
    }
    let page = request(
        b,
        "/secret/origin",
        json!({"session":session,"target":target}),
    )
    .await?;
    *origin = state::canonical_origin(page["origin"].as_str().ok_or("invalid_origin")?)
        .ok_or("invalid_origin")?;
    if input.get("origin").is_some() && input["origin"].as_str() != Some(origin.as_str()) {
        return Err("origin_changed");
    }
    let login = state
        .get_login_secret(name)
        .map_err(|_| "secret_unavailable")?
        .ok_or("unknown_secret")?;
    if !login.origins.is_empty() && !login.origins.iter().any(|allowed| allowed == origin) {
        return Err("origin_mismatch");
    }
    // Recheck after the network await: deletion/recreation or revocation invalidates the grant.
    if !state
        .browser_secret_active(&agent.name, name)
        .map_err(|_| "secret_unavailable")?
    {
        return Err("secret_not_allowed");
    }
    let value = match field {
        "username" => login.username,
        "password" => login.password,
        _ => {
            let seconds = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| "otp_unavailable")?
                .as_secs();
            crate::browser_redaction::totp(
                login.totp_seed.as_deref().ok_or("otp_unavailable")?,
                seconds,
                6,
                30,
            )
            .ok_or("otp_unavailable")?
        }
    };
    let result = request(
        b,
        "/secret/fill",
        json!({"session":session,"target":target,"origin":origin,"value":value}),
    )
    .await?;
    match result["status"].as_str() {
        Some("filled") => Ok("filled"),
        Some("origin_mismatch") => Err("origin_mismatch"),
        _ => Err("browser_refused"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn authorized_fill_is_body_only_and_outputs_are_value_free() {
        let dir = tempfile::tempdir().unwrap();
        let state = state::State::open(
            &dir.path().join("state.db"),
            Some(state::SecretKey::generate()),
        )
        .unwrap();
        let password = "fabricated-TV162-password-only";
        state
            .set_login_secret(
                "login",
                &state::LoginSecret {
                    username: "fabricated-user".into(),
                    password: password.into(),
                    totp_seed: None,
                    origins: vec!["https://example.com".into()],
                },
            )
            .unwrap();
        state
            .confirm_browser_secrets("tester", &["login".into()])
            .unwrap();
        let agent = schema::Agent::parse("tester", "label = \"Tester\"\npermissions {\n browser = true\n browser_secrets = [\"login\"]\n}\n").unwrap();
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let received = bodies.clone();
        let app = axum::Router::new()
            .route(
                "/secret/origin",
                axum::routing::post(|| async {
                    axum::Json(json!({"origin":"https://example.com"}))
                }),
            )
            .route(
                "/secret/fill",
                axum::routing::post(
                    move |headers: axum::http::HeaderMap, axum::Json(body): axum::Json<Value>| {
                        let received = received.clone();
                        async move {
                            assert!(headers.contains_key("x-vault-agent-signature"));
                            assert_eq!(headers["x-vault-agent"], "tester");
                            received.lock().unwrap().push(body);
                            axum::Json(json!({"status":"filled"}))
                        }
                    },
                ),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let b = BrowserBox {
            secret_browser: true,
            base: format!("http://{}", listener.local_addr().unwrap()),
            token: "fake-token".into(),
            profile: Some("tester".into()),
        };
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let input =
            json!({"secret":"login","field":"password","target":"#password","session":"default"});
        let out = authorized(&b, &state, &agent, &input).await;
        assert_eq!(out.content, "filled");
        assert!(!out.is_error);
        assert_eq!(bodies.lock().unwrap().len(), 1);
        assert_eq!(bodies.lock().unwrap()[0]["value"], password);
        assert_eq!(bodies.lock().unwrap()[0]["origin"], "https://example.com");
        for output in [
            &out.content,
            &out.summary.to_string(),
            out.trail.as_ref().unwrap(),
        ] {
            assert!(!output.contains(password));
        }
        assert_eq!(out.trail.as_ref().unwrap().lines().count(), 1);

        let held = call(&b, &state, &agent, "held", &input, false, true).await;
        assert!(!held.is_error);
        let proposal = held.event.unwrap().1;
        assert!(!proposal.to_string().contains(password));
        let refused = call(&b, &state, &agent, "mention", &input, true, true).await;
        assert!(refused.content.contains("confirmation_required"));
        let mut off = agent.clone();
        off.permissions.confirm.secrets = false;
        let capped = call(&b, &state, &off, "mention", &input, true, false).await;
        assert!(capped.content.contains("insufficient_scope"));
        assert_eq!(bodies.lock().unwrap().len(), 1);
        let mut moved = proposal;
        moved["origin"] = json!("https://old.example");
        assert!(
            authorized(&b, &state, &agent, &moved)
                .await
                .content
                .contains("origin_changed")
        );
        let mut unsafe_input = input.clone();
        unsafe_input["target"] = json!(format!("#{password}"));
        let unsafe_out = call(&b, &state, &agent, "unsafe", &unsafe_input, false, true).await;
        assert!(unsafe_out.event.is_none());
        assert!(unsafe_out.content.contains("unsafe_metadata"));
        assert!(!unsafe_out.trail.unwrap().contains(password));

        let mut denied = agent.clone();
        denied.permissions.browser_secrets.clear();
        assert!(authorized(&b, &state, &denied, &input).await.is_error);
        let missing = json!({"secret":"missing", "field":"password", "target":"#password", "session":"default"});
        let mut named_missing = agent.clone();
        named_missing
            .permissions
            .browser_secrets
            .push("missing".into());
        assert!(
            authorized(&b, &state, &named_missing, &missing)
                .await
                .content
                .contains("unknown_secret")
        );
        state
            .set_login_secret(
                "login",
                &state::LoginSecret {
                    username: "fabricated-user".into(),
                    password: password.into(),
                    totp_seed: None,
                    origins: vec!["https://other.example".into()],
                },
            )
            .unwrap();
        assert!(
            authorized(&b, &state, &agent, &input)
                .await
                .content
                .contains("origin_mismatch")
        );
        assert_eq!(bodies.lock().unwrap().len(), 1);
        let seed = "otpauth://totp/fixture?secret=JBSWY3DPEHPK3PXP&algorithm=SHA512&digits=8&period=60";
        state
            .set_login_secret(
                "login",
                &state::LoginSecret {
                    username: "fabricated-user".into(),
                    password: password.into(),
                    totp_seed: Some(seed.into()),
                    origins: vec![],
                },
            )
            .unwrap();
        let mut username = input.clone();
        username["field"] = json!("username");
        assert_eq!(
            authorized(&b, &state, &agent, &username).await.content,
            "filled"
        );
        assert_eq!(bodies.lock().unwrap()[1]["value"], "fabricated-user");
        let mut otp = input.clone();
        otp["field"] = json!("otp");
        let before = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let answer = authorized(&b, &state, &agent, &otp).await;
        assert_eq!(answer.content, "filled");
        let code = bodies.lock().unwrap()[2]["value"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!([before, before + 60].iter().any(|time| {
            crate::browser_redaction::totp(seed, *time, 6, 30).as_deref() == Some(code.as_str())
        }));
        assert_eq!(code.len(), 8);
        assert!(!answer.content.contains(&code));
        assert!(!answer.summary.to_string().contains(&code));
        assert!(!answer.trail.unwrap().contains(&code));
        let locked = state::State::open(&dir.path().join("state.db"), None).unwrap();
        assert!(authorized(&b, &locked, &agent, &input).await.is_error);
        state.revoke_browser_secrets("tester").unwrap();
        assert!(
            authorized(&b, &state, &agent, &input)
                .await
                .content
                .contains("secret_not_allowed")
        );
        assert_eq!(bodies.lock().unwrap().len(), 3);
        server.abort();
    }
}
