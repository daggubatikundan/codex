#![cfg(feature = "http-e2e-tests")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod common;
use codex_login::LoginServerOptions;
use codex_login::run_local_login_server_with_options;
use std::thread;
use std::time::Duration;
use tempfile::TempDir;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::method;
use wiremock::matchers::path;

async fn start_mock_oauth_server(behavior: MockBehavior) -> MockServer {
    let server = MockServer::start().await;

    match behavior {
        MockBehavior::Noop => {}
        MockBehavior::Success => {
            let id_token = make_fake_jwt(serde_json::json!({
                "https://api.openai.com/auth": {
                    "chatgpt_account_id": "acc-1",
                }
            }));
            let access_token = make_fake_jwt(serde_json::json!({
                "https://api.openai.com/auth": {
                    "organization_id": "org-1",
                    "project_id": "proj-1",
                    "completed_platform_onboarding": true,
                    "is_org_owner": false,
                    "chatgpt_plan_type": "plus"
                }
            }));
            let payload = serde_json::json!({
                "id_token": id_token,
                "access_token": access_token,
                "refresh_token": "refresh-1"
            });
            Mock::given(method("POST"))
                .and(path("/oauth/token"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("content-type", "application/json")
                        .set_body_json(payload),
                )
                .expect(1)
                .mount(&server)
                .await;
        }
        MockBehavior::SuccessTwice => {
            let id_token = make_fake_jwt(serde_json::json!({
                "https://api.openai.com/auth": {
                    "chatgpt_account_id": "acc-1",
                }
            }));
            let access_token = make_fake_jwt(serde_json::json!({
                "https://api.openai.com/auth": {
                    "organization_id": "org-1",
                    "project_id": "proj-1",
                    "completed_platform_onboarding": true,
                    "is_org_owner": false,
                    "chatgpt_plan_type": "plus"
                }
            }));
            let payload = serde_json::json!({
                "id_token": id_token,
                "access_token": access_token,
                "refresh_token": "refresh-1"
            });
            Mock::given(method("POST"))
                .and(path("/oauth/token"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("content-type", "application/json")
                        .set_body_json(payload),
                )
                .expect(2)
                .mount(&server)
                .await;
        }
        MockBehavior::SuccessNeedsSetup => {
            let id_token = make_fake_jwt(serde_json::json!({
                "https://api.openai.com/auth": {
                    "chatgpt_account_id": "acc-1",
                }
            }));
            let access_token = make_fake_jwt(serde_json::json!({
                "https://api.openai.com/auth": {
                    "organization_id": "org-2",
                    "project_id": "proj-2",
                    "completed_platform_onboarding": false,
                    "is_org_owner": true,
                    "chatgpt_plan_type": "pro"
                }
            }));
            let payload = serde_json::json!({
                "id_token": id_token,
                "access_token": access_token,
                "refresh_token": "refresh-1"
            });
            Mock::given(method("POST"))
                .and(path("/oauth/token"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("content-type", "application/json")
                        .set_body_json(payload),
                )
                .expect(1)
                .mount(&server)
                .await;
        }
        MockBehavior::SuccessIdClaimsOrgProject => {
            let id_token = make_fake_jwt(serde_json::json!({
                "https://api.openai.com/auth": {
                    "chatgpt_account_id": "acc-3",
                    "organization_id": "org-id",
                    "project_id": "proj-id",
                    "completed_platform_onboarding": true,
                    "is_org_owner": false
                }
            }));
            let access_token = make_fake_jwt(serde_json::json!({
                "https://api.openai.com/auth": {
                    "chatgpt_plan_type": "plus"
                }
            }));
            let payload = serde_json::json!({
                "id_token": id_token,
                "access_token": access_token,
                "refresh_token": "refresh-1"
            });
            Mock::given(method("POST"))
                .and(path("/oauth/token"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("content-type", "application/json")
                        .set_body_json(payload),
                )
                .expect(1)
                .mount(&server)
                .await;
        }
        MockBehavior::TokenError => {
            Mock::given(method("POST"))
                .and(path("/oauth/token"))
                .respond_with(ResponseTemplate::new(500))
                .expect(1)
                .mount(&server)
                .await;
        }
        MockBehavior::MissingOrgSkipExchange => {
            let id_token = make_fake_jwt(serde_json::json!({
                "https://api.openai.com/auth": {
                    "chatgpt_account_id": "acc-4"
                }
            }));
            let access_token = make_fake_jwt(serde_json::json!({
                "https://api.openai.com/auth": {
                    "chatgpt_plan_type": "plus"
                }
            }));
            let payload = serde_json::json!({
                "id_token": id_token,
                "access_token": access_token,
                "refresh_token": "refresh-4"
            });
            Mock::given(method("POST"))
                .and(path("/oauth/token"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("content-type", "application/json")
                        .set_body_json(payload),
                )
                .expect(1)
                .mount(&server)
                .await;
        }
    }

    server
}

#[derive(Clone, Copy)]
enum MockBehavior {
    Noop,
    Success,
    SuccessTwice,
    SuccessNeedsSetup,
    SuccessIdClaimsOrgProject,
    TokenError,
    MissingOrgSkipExchange,
    // Old token-exchange fallback behaviors removed
}

use common::make_fake_jwt;

fn spawn_login_server_and_wait(
    issuer: String,
    codex_home: &tempfile::TempDir,
    redeem_credits: bool,
) -> (std::thread::JoinHandle<std::io::Result<()>>, u16) {
    let (tx, rx) = std::sync::mpsc::channel();
    let opts = LoginServerOptions {
        codex_home: codex_home.path().to_path_buf(),
        client_id: "test-client".to_string(),
        issuer,
        port: 0,
        open_browser: false,
        redeem_credits,
        expose_state_endpoint: true,
        testing_timeout_secs: Some(5),
        verbose: false,
        #[cfg(feature = "http-e2e-tests")]
        port_sender: Some(tx),
    };

    let handle = thread::spawn(move || run_local_login_server_with_options(opts));
    let port = rx.recv().unwrap();
    wait_for_state_endpoint(port, Duration::from_secs(5));
    (handle, port)
}

fn http_get(url: &str) -> (u16, String, Option<String>) {
    let agent = ureq::AgentBuilder::new().redirects(0).build();
    match agent.get(url).call() {
        Ok(resp) => {
            let status = resp.status();
            let location = resp.header("Location").map(|s| s.to_string());
            let body = resp.into_string().unwrap_or_default();
            (status, body, location)
        }
        Err(ureq::Error::Status(code, resp)) => {
            let location = resp.header("Location").map(|s| s.to_string());
            let body = resp.into_string().unwrap_or_default();
            (code, body, location)
        }
        Err(err) => panic!("http error: {err}"),
    }
}

fn http_get_follow_redirect(url: &str) -> (u16, String) {
    let agent = ureq::AgentBuilder::new().redirects(5).build();
    match agent.get(url).call() {
        Ok(resp) => (resp.status(), resp.into_string().unwrap_or_default()),
        Err(ureq::Error::Status(code, resp)) => (code, resp.into_string().unwrap_or_default()),
        Err(err) => panic!("http error: {err}"),
    }
}

// 1) Happy path: writes auth.json and exits after /success
#[tokio::test]
async fn login_server_happy_path() {
    let server = start_mock_oauth_server(MockBehavior::SuccessTwice).await;

    let codex_home = TempDir::new().unwrap();
    let issuer = server.uri();
    let (handle, port) = spawn_login_server_and_wait(issuer, &codex_home, true);

    // Get state via test-only endpoint
    let state_url = format!("http://127.0.0.1:{port}/__test/state");
    let (_s, state, _) = http_get(&state_url);
    assert!(!state.is_empty());

    // Simulate callback
    let cb_url = format!("http://127.0.0.1:{port}/auth/callback?code=abc&state={state}");
    // First, capture redirect without following
    let (status, _body, location) = http_get(&cb_url);
    assert_eq!(status, 302);
    let location = location.expect("location header");
    assert!(location.contains("/success"));
    assert!(location.contains("needs_setup=false"));
    assert!(location.contains("plan_type=plus"));
    assert!(location.contains("org_id=org-1"));
    assert!(location.contains("project_id=proj-1"));
    // Now follow redirect (this will invoke the callback a second time)
    let (status, body) = http_get_follow_redirect(&cb_url);
    assert_eq!(status, 200);
    assert!(body.contains("Signed in to Codex CLI"));

    handle.join().unwrap().unwrap();

    // Verify auth.json written
    let auth_path = codex_login::get_auth_file(codex_home.path());
    let auth = codex_login::try_read_auth_json(&auth_path).unwrap();
    assert!(auth.openai_api_key.is_none());
    assert!(auth.tokens.as_ref().is_some());
    assert!(!auth.tokens.as_ref().unwrap().access_token.is_empty());
}
// 1b) needs_setup=true when onboarding incomplete and is_org_owner=true
#[tokio::test]
async fn login_server_needs_setup_true_and_params_present() {
    let server = start_mock_oauth_server(MockBehavior::SuccessNeedsSetup).await;

    let codex_home = TempDir::new().unwrap();
    let issuer = server.uri();
    let (handle, port) = spawn_login_server_and_wait(issuer, &codex_home, true);
    let state_url = format!("http://127.0.0.1:{port}/__test/state");
    let (_s, state, _) = http_get(&state_url);
    assert!(!state.is_empty());
    let cb_url = format!("http://127.0.0.1:{port}/auth/callback?code=abc&state={state}");
    let (status, _body, location) = http_get(&cb_url);
    assert_eq!(status, 302);
    let location = location.expect("location header");
    assert!(location.contains("needs_setup=true"));
    assert!(location.contains("plan_type=pro"));
    assert!(location.contains("org_id=org-2"));
    assert!(location.contains("project_id=proj-2"));
    let _ = ureq::get(&format!("http://127.0.0.1:{port}/success")).call();
    handle.join().unwrap().unwrap();
}

// 1c) org/project from ID token only should appear in redirect (fallback logic)
#[tokio::test]
async fn login_server_id_token_fallback_for_org_and_project() {
    let server = start_mock_oauth_server(MockBehavior::SuccessIdClaimsOrgProject).await;

    let codex_home = TempDir::new().unwrap();
    let issuer = server.uri();
    let (handle, port) = spawn_login_server_and_wait(issuer, &codex_home, true);
    let state_url = format!("http://127.0.0.1:{port}/__test/state");
    let (_s, state, _) = http_get(&state_url);
    let cb_url = format!("http://127.0.0.1:{port}/auth/callback?code=abc&state={state}");
    let (status, _body, location) = http_get(&cb_url);
    assert_eq!(status, 302);
    let location = location.expect("location header");
    assert!(location.contains("org_id=org-id"));
    assert!(location.contains("project_id=proj-id"));
    let _ = ureq::get(&format!("http://127.0.0.1:{port}/success")).call();
    handle.join().unwrap().unwrap();
}

// 1d) Missing org/project in claims -> skip token-exchange, persist tokens without API key, still success
#[tokio::test]
async fn login_server_skips_exchange_when_no_org_or_project() {
    let server = start_mock_oauth_server(MockBehavior::MissingOrgSkipExchange).await;

    let codex_home = TempDir::new().unwrap();
    let issuer = server.uri();
    let (handle, port) = spawn_login_server_and_wait(issuer, &codex_home, true);
    let state_url = format!("http://127.0.0.1:{port}/__test/state");
    let (_s, state, _) = http_get(&state_url);
    let cb_url = format!("http://127.0.0.1:{port}/auth/callback?code=abc&state={state}");
    let (status, _body, location) = http_get(&cb_url);
    assert_eq!(status, 302);
    let location = location.expect("location header");
    // No org_id/project_id in redirect
    assert!(!location.contains("org_id="));
    assert!(!location.contains("project_id="));
    let _ = ureq::get(&format!("http://127.0.0.1:{port}/success")).call();
    handle.join().unwrap().unwrap();

    // Verify auth.json OPENAI_API_KEY is null
    let auth_path = codex_login::get_auth_file(codex_home.path());
    let auth = codex_login::try_read_auth_json(&auth_path).unwrap();
    assert!(auth.openai_api_key.is_none());
}

//
// 2) State mismatch returns 400 and server stays up
#[tokio::test]
async fn login_server_state_mismatch() {
    let server = start_mock_oauth_server(MockBehavior::Noop).await;
    let codex_home = TempDir::new().unwrap();
    let issuer = server.uri();
    let (handle, port) = spawn_login_server_and_wait(issuer, &codex_home, false);

    let cb_url = format!("http://127.0.0.1:{port}/auth/callback?code=abc&state=wrong");
    let (status, body) = http_get_follow_redirect(&cb_url);
    assert_eq!(status, 400);
    assert!(body.contains("State parameter mismatch") || body.is_empty());

    // Stop server
    let _ = ureq::get(&format!("http://127.0.0.1:{port}/success")).call();
    handle.join().unwrap().unwrap();
}

// 3) Missing code returns 400
#[tokio::test]
async fn login_server_missing_code() {
    let server = start_mock_oauth_server(MockBehavior::Noop).await;
    let codex_home = TempDir::new().unwrap();
    let issuer = server.uri();
    let (handle, port) = spawn_login_server_and_wait(issuer, &codex_home, false);

    // Fetch state
    let state = ureq::get(&format!("http://127.0.0.1:{port}/__test/state"))
        .call()
        .expect("get state")
        .into_string()
        .unwrap();
    // Missing code
    let cb_url = format!("http://127.0.0.1:{port}/auth/callback?state={state}");
    let (status, _body) = http_get_follow_redirect(&cb_url);
    assert_eq!(status, 400);
    let _ = ureq::get(&format!("http://127.0.0.1:{port}/success")).call();
    handle.join().unwrap().unwrap();
}

// 4) Token endpoint error returns 500 (on code exchange) and server stays up
#[tokio::test]
async fn login_server_token_exchange_error() {
    let server = start_mock_oauth_server(MockBehavior::TokenError).await;
    let codex_home = TempDir::new().unwrap();
    let issuer = server.uri();
    let (handle, port) = spawn_login_server_and_wait(issuer, &codex_home, false);
    let state = ureq::get(&format!("http://127.0.0.1:{port}/__test/state"))
        .call()
        .expect("get state")
        .into_string()
        .unwrap();
    let cb_url = format!("http://127.0.0.1:{port}/auth/callback?code=abc&state={state}");
    let (status, _body) = http_get_follow_redirect(&cb_url);
    assert_eq!(status, 500);
    let _ = ureq::get(&format!("http://127.0.0.1:{port}/success")).call();
    handle.join().unwrap().unwrap();
}

// 5) Credit redemption errors do not block success
#[tokio::test]
async fn login_server_credit_redemption_best_effort() {
    // Mock behavior success for token endpoints
    let server = start_mock_oauth_server(MockBehavior::Success).await;
    let codex_home = TempDir::new().unwrap();
    let issuer = server.uri();
    let (handle, port) = spawn_login_server_and_wait(issuer, &codex_home, true);
    let state = ureq::get(&format!("http://127.0.0.1:{port}/__test/state"))
        .call()
        .expect("get state")
        .into_string()
        .unwrap();
    let cb_url = format!("http://127.0.0.1:{port}/auth/callback?code=abc&state={state}");
    let (status, _body) = http_get_follow_redirect(&cb_url);
    assert_eq!(status, 200);
    handle.join().unwrap().unwrap();
    // auth.json exists
    assert!(codex_login::get_auth_file(codex_home.path()).exists());
}

fn wait_for_state_endpoint(port: u16, timeout: Duration) {
    let start = std::time::Instant::now();
    loop {
        if start.elapsed() > timeout {
            panic!("server did not expose __test/state within timeout");
        }
        if let Ok(resp) = ureq::get(&format!("http://127.0.0.1:{port}/__test/state")).call() {
            if resp.status() == 200 {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
