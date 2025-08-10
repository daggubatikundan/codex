pub use crate::token_data::TokenData;

mod auth;
mod auth_store;
mod entrypoints;
mod pkce;
mod refresh;
mod server;
mod success_url;
mod token_data;

pub use auth::AuthMode;
pub use auth::CodexAuth;
pub use auth_store::AuthDotJson;
pub use auth_store::get_auth_file;
pub use auth_store::login_with_api_key;
pub use auth_store::logout;
pub use auth_store::try_read_auth_json;
pub use entrypoints::SpawnedLogin;
pub use entrypoints::login_with_chatgpt;
pub use entrypoints::spawn_login_with_chatgpt;
pub use server::HeadlessOutcome;
pub use server::Http;
pub use server::LoginServerOptions;
pub use server::process_callback_headless;
pub use server::run_local_login_server_with_options;

pub(crate) const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const OPENAI_API_KEY_ENV_VAR: &str = "OPENAI_API_KEY";
pub const EXIT_CODE_WHEN_ADDRESS_ALREADY_IN_USE: i32 = 13;

#[cfg(test)]
mod lib_tests;
