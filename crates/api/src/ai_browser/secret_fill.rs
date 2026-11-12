//! Server-owned fill authorization. TV-164 will call this after confirmation.
use super::*;

#[allow(dead_code)] // Wired to the confirmation Apply path by TV-164, never a model flag.
pub(crate) async fn authorized(
    _b: &BrowserBox,
    _state: &state::State,
    _agent: &schema::agents::Agent,
    _input: &Value,
) -> BrowserOut {
    BrowserOut::error("not_implemented")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn authorized_fill_is_body_only_and_outputs_are_value_free() {
        let dir = tempfile::tempdir().unwrap();
        let state = state::State::open(&dir.path().join("state.db"), Some(state::SecretKey::generate())).unwrap();
        let password = "fabricated-TV162-password-only";
        state.set_login_secret("login", &state::LoginSecret {
            username: "fabricated-user".into(), password: password.into(), totp_seed: None,
            origins: vec!["https://example.com".into()],
        }).unwrap();
        state.confirm_browser_secrets("tester", &["login".into()]).unwrap();
        let agent = schema::agents::Agent::parse("tester", "label = \"Tester\"\npermissions {\n browser = true\n browser_secrets = [\"login\"]\n}\n").unwrap();
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let received = bodies.clone();
        let app = axum::Router::new()
            .route("/secret/origin", axum::routing::post(|| async { axum::Json(json!({"origin":"https://example.com"})) }))
            .route("/secret/fill", axum::routing::post(move |headers: axum::http::HeaderMap, axum::Json(body): axum::Json<Value>| {
                let received = received.clone();
                async move {
                    assert!(headers.contains_key("x-vault-agent-signature"));
                    assert_eq!(headers["x-vault-agent"], "tester");
                    received.lock().unwrap().push(body);
                    axum::Json(json!({"status":"filled"}))
                }
            }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let b = BrowserBox { base: format!("http://{}", listener.local_addr().unwrap()), token:"fake-token".into(), profile:Some("tester".into()) };
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        let input = json!({"secret":"login","field":"password","target":"#password","session":"default"});
        let out = authorized(&b, &state, &agent, &input).await;
        assert_eq!(out.content, "filled");
        assert!(!out.is_error);
        assert_eq!(bodies.lock().unwrap().len(), 1);
        assert_eq!(bodies.lock().unwrap()[0]["value"], password);
        assert_eq!(bodies.lock().unwrap()[0]["origin"], "https://example.com");
        for output in [&out.content, &out.summary.to_string(), out.trail.as_ref().unwrap()] {
            assert!(!output.contains(password));
        }
        assert_eq!(out.trail.as_ref().unwrap().lines().count(), 1);

        let mut denied = agent.clone();
        denied.permissions.browser_secrets.clear();
        assert!(authorized(&b, &state, &denied, &input).await.is_error);
        let missing = json!({"secret":"missing", "field":"password", "target":"#password", "session":"default"});
        assert!(authorized(&b, &state, &agent, &missing).await.is_error);
        state.set_login_secret("login", &state::LoginSecret {
            username: "fabricated-user".into(), password: password.into(), totp_seed: None,
            origins: vec!["https://other.example".into()],
        }).unwrap();
        assert!(authorized(&b, &state, &agent, &input).await.is_error);
        assert_eq!(bodies.lock().unwrap().len(), 1);
        server.abort();
    }
}
