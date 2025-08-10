#![expect(clippy::expect_used, clippy::unwrap_used)]
use super::*;
use crate::auth::AuthMode;
use crate::auth::CodexAuth;
use crate::auth::load_auth;
use crate::auth_store::AuthDotJson;
use crate::auth_store::get_auth_file;
use crate::auth_store::logout;
use crate::token_data::IdTokenInfo;
use crate::token_data::KnownPlan;
use crate::token_data::PlanType;
use crate::token_data::parse_id_token;
use base64::Engine;
use pretty_assertions::assert_eq;
use serde::Serialize;
use serde_json::json;
use std::path::Path;
use tempfile::tempdir;

const LAST_REFRESH: &str = "2025-08-06T20:41:36.232376Z";

#[test]
fn writes_api_key_and_loads_auth() {
    let dir = tempdir().unwrap();
    crate::auth_store::login_with_api_key(dir.path(), "sk-test-key").unwrap();
    let auth = load_auth(dir.path(), false).unwrap().unwrap();
    assert_eq!(auth.mode, AuthMode::ApiKey);
    assert_eq!(auth.api_key.as_deref(), Some("sk-test-key"));
}

#[test]
fn loads_from_env_var_if_env_var_exists() {
    let dir = tempdir().unwrap();
    let env_var = std::env::var(crate::OPENAI_API_KEY_ENV_VAR);
    if let Ok(env_var) = env_var {
        let auth = load_auth(dir.path(), true).unwrap().unwrap();
        assert_eq!(auth.mode, AuthMode::ApiKey);
        assert_eq!(auth.api_key, Some(env_var));
    }
}

#[tokio::test]
async fn pro_account_with_no_api_key_uses_chatgpt_auth() {
    let codex_home = tempdir().unwrap();
    write_auth_file(
        AuthFileParams {
            openai_api_key: None,
            chatgpt_plan_type: "pro".to_string(),
        },
        codex_home.path(),
    )
    .expect("failed to write auth file");

    let CodexAuth {
        api_key,
        mode,
        auth_dot_json,
        auth_file: _,
    } = load_auth(codex_home.path(), false).unwrap().unwrap();
    assert_eq!(None, api_key);
    assert_eq!(AuthMode::ChatGPT, mode);

    let guard = auth_dot_json.lock().unwrap();
    let auth_dot_json = guard.as_ref().expect("AuthDotJson should exist");
    assert_eq!(
        &AuthDotJson {
            openai_api_key: None,
            tokens: Some(TokenData {
                id_token: IdTokenInfo {
                    email: Some("user@example.com".to_string()),
                    chatgpt_plan_type: Some(PlanType::Known(KnownPlan::Pro)),
                },
                access_token: "test-access-token".to_string(),
                refresh_token: "test-refresh-token".to_string(),
                account_id: None,
            }),
            last_refresh: Some(
                chrono::DateTime::parse_from_rfc3339(LAST_REFRESH)
                    .unwrap()
                    .with_timezone(&chrono::Utc),
            ),
        },
        auth_dot_json
    )
}

/// Even if the OPENAI_API_KEY is set in auth.json, if the plan is not in
/// [`TokenData::is_plan_that_should_use_api_key`], it should use
/// [`AuthMode::ChatGPT`].
#[tokio::test]
async fn pro_account_with_api_key_still_uses_chatgpt_auth() {
    let codex_home = tempdir().unwrap();
    write_auth_file(
        AuthFileParams {
            openai_api_key: Some("sk-test-key".to_string()),
            chatgpt_plan_type: "pro".to_string(),
        },
        codex_home.path(),
    )
    .expect("failed to write auth file");

    let CodexAuth {
        api_key,
        mode,
        auth_dot_json,
        auth_file: _,
    } = load_auth(codex_home.path(), false).unwrap().unwrap();
    assert_eq!(None, api_key);
    assert_eq!(AuthMode::ChatGPT, mode);

    let guard = auth_dot_json.lock().unwrap();
    let auth_dot_json = guard.as_ref().expect("AuthDotJson should exist");
    assert_eq!(
        &AuthDotJson {
            openai_api_key: None,
            tokens: Some(TokenData {
                id_token: IdTokenInfo {
                    email: Some("user@example.com".to_string()),
                    chatgpt_plan_type: Some(PlanType::Known(KnownPlan::Pro)),
                },
                access_token: "test-access-token".to_string(),
                refresh_token: "test-refresh-token".to_string(),
                account_id: None,
            }),
            last_refresh: Some(
                chrono::DateTime::parse_from_rfc3339(LAST_REFRESH)
                    .unwrap()
                    .with_timezone(&chrono::Utc),
            ),
        },
        auth_dot_json
    )
}

/// If the OPENAI_API_KEY is set in auth.json and it is an enterprise
/// account, then it should use [`AuthMode::ApiKey`].
#[tokio::test]
async fn enterprise_account_with_api_key_uses_chatgpt_auth() {
    let codex_home = tempdir().unwrap();
    write_auth_file(
        AuthFileParams {
            openai_api_key: Some("sk-test-key".to_string()),
            chatgpt_plan_type: "enterprise".to_string(),
        },
        codex_home.path(),
    )
    .expect("failed to write auth file");

    let CodexAuth {
        api_key,
        mode,
        auth_dot_json,
        auth_file: _,
    } = load_auth(codex_home.path(), false).unwrap().unwrap();
    assert_eq!(Some("sk-test-key".to_string()), api_key);
    assert_eq!(AuthMode::ApiKey, mode);

    let guard = auth_dot_json.lock().expect("should unwrap");
    assert!(guard.is_none(), "auth_dot_json should be None");
}

struct AuthFileParams {
    openai_api_key: Option<String>,
    chatgpt_plan_type: String,
}

fn write_auth_file(params: AuthFileParams, codex_home: &Path) -> std::io::Result<()> {
    let auth_file = get_auth_file(codex_home);
    #[derive(Serialize)]
    struct Header {
        alg: &'static str,
        typ: &'static str,
    }
    let header = Header {
        alg: "none",
        typ: "JWT",
    };
    let payload = serde_json::json!({
        "email": "user@example.com",
        "email_verified": true,
        "https://api.openai.com/auth": {
            "chatgpt_account_id": "bc3618e3-489d-4d49-9362-1561dc53ba53",
            "chatgpt_plan_type": params.chatgpt_plan_type,
            "chatgpt_user_id": "user-12345",
            "user_id": "user-12345",
        }
    });
    let b64 = |b: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b);
    let header_b64 = b64(&serde_json::to_vec(&header)?);
    let payload_b64 = b64(&serde_json::to_vec(&payload)?);
    let signature_b64 = b64(b"sig");
    let fake_jwt = format!("{header_b64}.{payload_b64}.{signature_b64}");

    let auth_json_data = json!({
        "OPENAI_API_KEY": params.openai_api_key,
        "tokens": {
            "id_token": fake_jwt,
            "access_token": "test-access-token",
            "refresh_token": "test-refresh-token"
        },
        "last_refresh": LAST_REFRESH,
    });
    let auth_json = serde_json::to_string_pretty(&auth_json_data)?;
    std::fs::write(auth_file, auth_json)
}

#[test]
fn id_token_info_handles_missing_fields() {
    // Payload without email or plan should yield None values.
    let header = serde_json::json!({"alg": "none", "typ": "JWT"});
    let payload = serde_json::json!({"sub": "123"});
    let header_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(&header).unwrap());
    let payload_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(&payload).unwrap());
    let signature_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"sig");
    let jwt = format!("{header_b64}.{payload_b64}.{signature_b64}");

    let info = parse_id_token(&jwt).expect("should parse");
    assert!(info.email.is_none());
    assert!(info.chatgpt_plan_type.is_none());
}

#[tokio::test]
async fn loads_api_key_from_auth_json() {
    let dir = tempdir().unwrap();
    let auth_file = dir.path().join("auth.json");
    std::fs::write(
        auth_file,
        r#"
        {
            "OPENAI_API_KEY": "sk-test-key",
            "tokens": null,
            "last_refresh": null
        }
        "#,
    )
    .unwrap();

    let auth = load_auth(dir.path(), false).unwrap().unwrap();
    assert_eq!(auth.mode, AuthMode::ApiKey);
    assert_eq!(auth.api_key, Some("sk-test-key".to_string()));

    assert!(auth.get_token_data().await.is_err());
}

#[test]
fn logout_removes_auth_file() -> Result<(), std::io::Error> {
    let dir = tempdir()?;
    crate::auth_store::login_with_api_key(dir.path(), "sk-test-key")?;
    assert!(dir.path().join("auth.json").exists());
    let removed = logout(dir.path())?;
    assert!(removed);
    assert!(!dir.path().join("auth.json").exists());
    Ok(())
}
