#![expect(clippy::unwrap_used)]
use codex_login::LoginServerOptions;
use codex_login::process_callback_headless;
use serde_json::json;
mod common;
use std::cell::RefCell;
use std::collections::VecDeque;
use tempfile::TempDir;

type FormCapture = (String, Vec<(String, String)>);

#[derive(Default)]
struct MockHttp {
    forms: RefCell<Vec<FormCapture>>,
    jsons: RefCell<Vec<(String, serde_json::Value)>>,
    replies: RefCell<VecDeque<serde_json::Value>>,
}

impl MockHttp {
    fn queue(&self, val: serde_json::Value) {
        self.replies.borrow_mut().push_back(val);
    }
}

impl codex_login::Http for MockHttp {
    fn post_form(
        &self,
        url: &str,
        form: &[(String, String)],
    ) -> std::io::Result<serde_json::Value> {
        self.forms
            .borrow_mut()
            .push((url.to_string(), form.to_vec()));
        self.replies
            .borrow_mut()
            .pop_front()
            .ok_or_else(|| std::io::Error::other("no reply"))
    }

    fn post_json(&self, url: &str, body: &serde_json::Value) -> std::io::Result<serde_json::Value> {
        self.jsons
            .borrow_mut()
            .push((url.to_string(), body.clone()));
        self.replies
            .borrow_mut()
            .pop_front()
            .ok_or_else(|| std::io::Error::other("no reply"))
    }
}

use common::make_fake_jwt;

fn default_opts(tmp: &TempDir) -> LoginServerOptions {
    LoginServerOptions {
        codex_home: tmp.path().to_path_buf(),
        client_id: "test-client".into(),
        issuer: "http://auth.local".into(),
        port: 1455,
        open_browser: false,
        redeem_credits: true,
        expose_state_endpoint: false,
        testing_timeout_secs: None,
        verbose: false,
        #[cfg(feature = "http-e2e-tests")]
        port_sender: None,
    }
}

// 1) Success flow writes file and returns success URL
#[test]
fn headless_success_writes_auth_and_url() {
    let tmp = TempDir::new().unwrap();
    let opts = default_opts(&tmp);
    let http = MockHttp::default();
    // Code exchange response
    http.queue(json!({
        "id_token": make_fake_jwt(json!({"https://api.openai.com/auth": {"chatgpt_account_id": "acc"}})),
        "access_token": make_fake_jwt(json!({"https://api.openai.com/auth": {"organization_id": "org","project_id": "proj","completed_platform_onboarding": true, "is_org_owner": false, "chatgpt_plan_type": "plus"}})),
        "refresh_token": "r1"
    }));
    // Credits redeem
    http.queue(json!({"granted_chatgpt_subscriber_api_credits": 5}));

    let outcome =
        process_callback_headless(&opts, "state", "state", Some("code"), "ver", &http).unwrap();
    assert!(outcome.success_url.contains("/success"));
    let auth_path = codex_login::get_auth_file(tmp.path());
    let auth = codex_login::try_read_auth_json(&auth_path).unwrap();
    assert!(auth.openai_api_key.is_none());
}

// 2) State mismatch errors
#[test]
fn headless_state_mismatch() {
    let tmp = TempDir::new().unwrap();
    let opts = default_opts(&tmp);
    let http = MockHttp::default();
    let err = process_callback_headless(&opts, "state", "wrong", Some("code"), "ver", &http)
        .err()
        .unwrap();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

// 3) Missing code errors
#[test]
fn headless_missing_code() {
    let tmp = TempDir::new().unwrap();
    let opts = default_opts(&tmp);
    let http = MockHttp::default();
    let err = process_callback_headless(&opts, "state", "state", None, "ver", &http)
        .err()
        .unwrap();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

// 4) Token endpoint failure propagates error
#[test]
fn headless_token_endpoint_failure() {
    let tmp = TempDir::new().unwrap();
    let opts = default_opts(&tmp);
    let http = MockHttp::default();
    // no replies queued -> will error
    let err = process_callback_headless(&opts, "state", "state", Some("code"), "ver", &http)
        .err()
        .unwrap();
    assert_eq!(err.kind(), std::io::ErrorKind::Other);
}

// 5) Credit redemption best-effort: even if it errors, success persists
#[test]
fn headless_credit_redemption_best_effort() {
    let tmp = TempDir::new().unwrap();
    let mut opts = default_opts(&tmp);
    opts.redeem_credits = true;
    let http = MockHttp::default();
    // Code exchange
    http.queue(json!({
        "id_token": make_fake_jwt(json!({"https://api.openai.com/auth": {"chatgpt_account_id": "acc"}})),
        "access_token": make_fake_jwt(json!({"https://api.openai.com/auth": {"organization_id": "org","project_id": "proj","completed_platform_onboarding": false, "is_org_owner": true, "chatgpt_plan_type": "pro"}})),
        "refresh_token": "r1"
    }));
    // Credits redeem: simulate error by not queuing a third response; the mock will error internally
    let outcome =
        process_callback_headless(&opts, "state", "state", Some("code"), "ver", &http).unwrap();
    assert!(outcome.success_url.contains("needs_setup=true"));
    assert!(tmp.path().join("auth.json").exists());
}

// 6) ID-token fallback for org/project/flags
#[test]
fn headless_id_token_fallback_for_org_and_project() {
    let tmp = TempDir::new().unwrap();
    let opts = default_opts(&tmp);
    let http = MockHttp::default();
    // Code exchange: put org/project/flags into ID token; plan_type into access
    http.queue(json!({
        "id_token": make_fake_jwt(json!({
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acc",
                "organization_id": "id-org",
                "project_id": "id-proj",
                "completed_platform_onboarding": true,
                "is_org_owner": false
            }
        })),
        "access_token": make_fake_jwt(json!({
            "https://api.openai.com/auth": {
                "chatgpt_plan_type": "plus"
            }
        })),
        "refresh_token": "r1"
    }));
    // Credits redeem
    http.queue(json!({"granted_chatgpt_subscriber_api_credits": 0}));

    let outcome =
        process_callback_headless(&opts, "state", "state", Some("code"), "ver", &http).unwrap();
    assert!(outcome.success_url.contains("org_id=id-org"));
    assert!(outcome.success_url.contains("project_id=id-proj"));
}
