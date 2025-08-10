use base64::Engine;
use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Default)]
pub struct TokenData {
    /// Flat info parsed from the JWT in auth.json.
    #[serde(deserialize_with = "deserialize_id_token")]
    pub id_token: IdTokenInfo,

    /// This is a JWT.
    pub access_token: String,

    pub refresh_token: String,

    pub account_id: Option<String>,
}

impl TokenData {
    pub(crate) fn is_plan_that_should_use_api_key(&self) -> bool {
        self.id_token
            .chatgpt_plan_type
            .as_ref()
            .is_none_or(|plan| plan.is_plan_that_should_use_api_key())
    }
}

/// Flat subset of useful claims in id_token from auth.json.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct IdTokenInfo {
    pub email: Option<String>,
    pub(crate) chatgpt_plan_type: Option<PlanType>,
}

impl IdTokenInfo {
    pub fn get_chatgpt_plan_type(&self) -> Option<String> {
        self.chatgpt_plan_type.as_ref().map(|t| match t {
            PlanType::Known(plan) => format!("{plan:?}"),
            PlanType::Unknown(s) => s.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum PlanType {
    Known(KnownPlan),
    Unknown(String),
}

impl PlanType {
    fn is_plan_that_should_use_api_key(&self) -> bool {
        match self {
            Self::Known(known) => {
                use KnownPlan::*;
                !matches!(known, Free | Plus | Pro | Team)
            }
            Self::Unknown(_) => {
                // Unknown plans should use the API key.
                true
            }
        }
    }

    pub fn as_string(&self) -> String {
        match self {
            Self::Known(known) => format!("{known:?}").to_lowercase(),
            Self::Unknown(s) => s.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum KnownPlan {
    Free,
    Plus,
    Pro,
    Team,
    Business,
    Enterprise,
    Edu,
}

#[derive(Deserialize)]
struct IdClaims {
    #[serde(default)]
    email: Option<String>,
    #[serde(rename = "https://api.openai.com/auth", default)]
    auth: Option<AuthClaims>,
}

#[derive(Deserialize)]
struct AuthClaims {
    #[serde(default)]
    chatgpt_plan_type: Option<PlanType>,
}

#[derive(Debug, Error)]
pub enum IdTokenInfoError {
    #[error("invalid ID token format")]
    InvalidFormat,
    #[error(transparent)]
    Base64(#[from] base64::DecodeError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub(crate) fn parse_id_token(id_token: &str) -> Result<IdTokenInfo, IdTokenInfoError> {
    // JWT format: header.payload.signature
    let mut parts = id_token.split('.');
    let (_header_b64, payload_b64, _sig_b64) = match (parts.next(), parts.next(), parts.next()) {
        (Some(h), Some(p), Some(s)) if !h.is_empty() && !p.is_empty() && !s.is_empty() => (h, p, s),
        _ => return Err(IdTokenInfoError::InvalidFormat),
    };

    let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload_b64)?;
    let claims: IdClaims = serde_json::from_slice(&payload_bytes)?;

    Ok(IdTokenInfo {
        email: claims.email,
        chatgpt_plan_type: claims.auth.and_then(|a| a.chatgpt_plan_type),
    })
}

fn deserialize_id_token<'de, D>(deserializer: D) -> Result<IdTokenInfo, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    parse_id_token(&s).map_err(serde::de::Error::custom)
}

// -------- Helpers for parsing OpenAI auth claims from arbitrary JWTs --------

#[derive(Default, Deserialize)]
struct AuthOuterClaims {
    #[serde(rename = "https://api.openai.com/auth", default)]
    auth: Option<AuthInnerClaims>,
}

#[derive(Default, Deserialize, Clone)]
struct AuthInnerClaims {
    #[serde(default)]
    chatgpt_account_id: Option<String>,
    #[serde(default)]
    organization_id: Option<String>,
    #[serde(default)]
    project_id: Option<String>,
    #[serde(default)]
    completed_platform_onboarding: Option<bool>,
    #[serde(default)]
    is_org_owner: Option<bool>,
    #[serde(default)]
    chatgpt_plan_type: Option<PlanType>,
}

fn decode_jwt_payload(token: &str) -> Option<Vec<u8>> {
    let mut parts = token.split('.');
    let _header = parts.next();
    let payload_b64 = parts.next();
    let _sig = parts.next();
    payload_b64.and_then(|p| {
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(p)
            .ok()
    })
}

fn parse_auth_inner_claims(token: &str) -> AuthInnerClaims {
    decode_jwt_payload(token)
        .and_then(|bytes| serde_json::from_slice::<AuthOuterClaims>(&bytes).ok())
        .and_then(|o| o.auth)
        .unwrap_or_default()
}

/// Extracts commonly used claims from ID and access tokens.
/// - account_id is taken from the ID token.
/// - org_id/project_id prefer ID token, falling back to access token.
/// - plan_type comes from the access token (as lowercase string).
/// - needs_setup is computed from (completed_platform_onboarding, is_org_owner)
pub(crate) fn extract_login_context_from_tokens(
    id_token: &str,
    access_token: &str,
) -> (
    Option<String>, // account_id
    Option<String>, // org_id
    Option<String>, // project_id
    bool,           // needs_setup
    Option<String>, // plan_type
) {
    let id_inner = parse_auth_inner_claims(id_token);
    let access_inner = parse_auth_inner_claims(access_token);

    let account_id = id_inner.chatgpt_account_id.clone();
    let org_id = id_inner
        .organization_id
        .clone()
        .or_else(|| access_inner.organization_id.clone());
    let project_id = id_inner
        .project_id
        .clone()
        .or_else(|| access_inner.project_id.clone());

    let completed_onboarding = id_inner
        .completed_platform_onboarding
        .or(access_inner.completed_platform_onboarding)
        .unwrap_or(false);
    let is_org_owner = id_inner
        .is_org_owner
        .or(access_inner.is_org_owner)
        .unwrap_or(false);
    let needs_setup = !completed_onboarding && is_org_owner;

    let plan_type = access_inner
        .chatgpt_plan_type
        .as_ref()
        .map(PlanType::as_string);

    (account_id, org_id, project_id, needs_setup, plan_type)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[test]
    #[expect(clippy::expect_used, clippy::unwrap_used)]
    fn id_token_info_parses_email_and_plan() {
        // Build a fake JWT with a URL-safe base64 payload containing email and plan.
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
            "https://api.openai.com/auth": {
                "chatgpt_plan_type": "pro"
            }
        });

        fn b64url_no_pad(bytes: &[u8]) -> String {
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
        }

        let header_b64 = b64url_no_pad(&serde_json::to_vec(&header).unwrap());
        let payload_b64 = b64url_no_pad(&serde_json::to_vec(&payload).unwrap());
        let signature_b64 = b64url_no_pad(b"sig");
        let fake_jwt = format!("{header_b64}.{payload_b64}.{signature_b64}");

        let info = parse_id_token(&fake_jwt).expect("should parse");
        assert_eq!(info.email.as_deref(), Some("user@example.com"));
        assert_eq!(
            info.chatgpt_plan_type,
            Some(PlanType::Known(KnownPlan::Pro))
        );
    }
}
