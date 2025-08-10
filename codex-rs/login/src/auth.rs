use chrono::Utc;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use crate::auth_store::AuthDotJson;
use crate::auth_store::get_auth_file;
use crate::auth_store::try_read_auth_json;
use crate::auth_store::update_tokens;
use crate::refresh::try_refresh_token;
use crate::token_data::TokenData;

#[derive(Clone, Debug, PartialEq, Copy)]
pub enum AuthMode {
    ApiKey,
    ChatGPT,
}

#[derive(Debug, Clone)]
pub struct CodexAuth {
    pub mode: AuthMode,

    pub(crate) api_key: Option<String>,
    pub(crate) auth_dot_json: Arc<Mutex<Option<AuthDotJson>>>,
    pub(crate) auth_file: PathBuf,
}

impl PartialEq for CodexAuth {
    fn eq(&self, other: &Self) -> bool {
        self.mode == other.mode
    }
}

impl CodexAuth {
    pub fn from_api_key(api_key: &str) -> Self {
        Self {
            api_key: Some(api_key.to_owned()),
            mode: AuthMode::ApiKey,
            auth_file: PathBuf::new(),
            auth_dot_json: Arc::new(Mutex::new(None)),
        }
    }

    /// Loads from auth.json or OPENAI_API_KEY
    pub fn from_codex_home(codex_home: &Path) -> std::io::Result<Option<CodexAuth>> {
        load_auth(codex_home, true)
    }

    pub async fn get_token_data(&self) -> Result<TokenData, std::io::Error> {
        let auth_dot_json: Option<AuthDotJson> = self.get_current_auth_json();
        match auth_dot_json {
            Some(AuthDotJson {
                tokens: Some(mut tokens),
                last_refresh: Some(last_refresh),
                ..
            }) => {
                if last_refresh < Utc::now() - chrono::Duration::days(28) {
                    let refresh_response = tokio::time::timeout(
                        Duration::from_secs(60),
                        try_refresh_token(tokens.refresh_token.clone()),
                    )
                    .await
                    .map_err(|_| {
                        std::io::Error::other("timed out while refreshing OpenAI API key")
                    })?
                    .map_err(std::io::Error::other)?;

                    let updated_auth_dot_json = update_tokens(
                        &self.auth_file,
                        refresh_response.id_token,
                        refresh_response.access_token,
                        refresh_response.refresh_token,
                    )?;

                    tokens = updated_auth_dot_json
                        .tokens
                        .clone()
                        .ok_or(std::io::Error::other(
                            "Token data is not available after refresh.",
                        ))?;

                    #[expect(clippy::unwrap_used)]
                    let mut auth_lock = self.auth_dot_json.lock().unwrap();
                    *auth_lock = Some(updated_auth_dot_json);
                }

                Ok(tokens)
            }
            _ => Err(std::io::Error::other("Token data is not available.")),
        }
    }

    pub async fn get_token(&self) -> Result<String, std::io::Error> {
        match self.mode {
            AuthMode::ApiKey => Ok(self.api_key.clone().unwrap_or_default()),
            AuthMode::ChatGPT => {
                let id_token = self.get_token_data().await?.access_token;

                Ok(id_token)
            }
        }
    }

    pub fn get_account_id(&self) -> Option<String> {
        self.get_current_token_data()
            .and_then(|t| t.account_id.clone())
    }

    pub fn get_plan_type(&self) -> Option<String> {
        self.get_current_token_data()
            .and_then(|t| t.id_token.chatgpt_plan_type.as_ref().map(|p| p.as_string()))
    }

    fn get_current_auth_json(&self) -> Option<AuthDotJson> {
        #[expect(clippy::unwrap_used)]
        self.auth_dot_json.lock().unwrap().clone()
    }

    fn get_current_token_data(&self) -> Option<TokenData> {
        self.get_current_auth_json().and_then(|t| t.tokens.clone())
    }

    /// Consider this private to integration tests.
    pub fn create_dummy_chatgpt_auth_for_testing() -> Self {
        let auth_dot_json = AuthDotJson {
            openai_api_key: None,
            tokens: Some(TokenData {
                id_token: Default::default(),
                access_token: "Access Token".to_string(),
                refresh_token: "test".to_string(),
                account_id: Some("account_id".to_string()),
            }),
            last_refresh: Some(Utc::now()),
        };

        let auth_dot_json = Arc::new(Mutex::new(Some(auth_dot_json)));
        Self {
            api_key: None,
            mode: AuthMode::ChatGPT,
            auth_file: PathBuf::new(),
            auth_dot_json,
        }
    }
}

pub(crate) fn load_auth(
    codex_home: &Path,
    include_env_var: bool,
) -> std::io::Result<Option<CodexAuth>> {
    // First, check to see if there is a valid auth.json file. If not, we fall
    // back to AuthMode::ApiKey using the OPENAI_API_KEY environment variable
    let auth_file = get_auth_file(codex_home);
    let auth_dot_json = match try_read_auth_json(&auth_file) {
        Ok(auth) => auth,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && include_env_var => {
            return match read_openai_api_key_from_env() {
                Some(api_key) => Ok(Some(CodexAuth::from_api_key(&api_key))),
                None => Ok(None),
            };
        }
        // Though if auth.json exists but is malformed, do not fall back to the
        // env var because the user may be expecting to use AuthMode::ChatGPT.
        Err(e) => {
            return Err(e);
        }
    };

    let AuthDotJson {
        openai_api_key: auth_json_api_key,
        tokens,
        last_refresh,
    } = auth_dot_json;

    // If the auth.json has an API key AND does not appear to be on a plan that
    // should prefer AuthMode::ChatGPT, use AuthMode::ApiKey.
    if let Some(api_key) = &auth_json_api_key {
        match &tokens {
            Some(tokens) => {
                if tokens.is_plan_that_should_use_api_key() {
                    return Ok(Some(CodexAuth::from_api_key(api_key)));
                }
            }
            None => {
                // Let's assume they are trying to use their API key.
                return Ok(Some(CodexAuth::from_api_key(api_key)));
            }
        }
    }

    Ok(Some(CodexAuth {
        api_key: None,
        mode: AuthMode::ChatGPT,
        auth_file,
        auth_dot_json: Arc::new(Mutex::new(Some(AuthDotJson {
            openai_api_key: None,
            tokens,
            last_refresh,
        }))),
    }))
}

fn read_openai_api_key_from_env() -> Option<String> {
    std::env::var(crate::OPENAI_API_KEY_ENV_VAR)
        .ok()
        .filter(|s| !s.is_empty())
}
