use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use std::fs::File;
use std::fs::OpenOptions;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::path::PathBuf;

use crate::token_data::TokenData;

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct AuthDotJson {
    #[serde(rename = "OPENAI_API_KEY")]
    pub openai_api_key: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<TokenData>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_refresh: Option<DateTime<Utc>>,
}

pub fn get_auth_file(codex_home: &Path) -> PathBuf {
    codex_home.join("auth.json")
}

/// Delete the auth.json file inside `codex_home` if it exists. Returns `Ok(true)`
/// if a file was removed, `Ok(false)` if no auth file was present.
pub fn logout(codex_home: &Path) -> std::io::Result<bool> {
    let auth_file = get_auth_file(codex_home);
    match std::fs::remove_file(&auth_file) {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err),
    }
}

/// Attempt to read and refresh the `auth.json` file in the given `CODEX_HOME` directory.
/// Returns the full AuthDotJson structure after refreshing if necessary.
pub fn try_read_auth_json(auth_file: &Path) -> std::io::Result<AuthDotJson> {
    let mut file = File::open(auth_file)?;
    let mut contents = String::new();
    use std::io::Read as _;
    file.read_to_string(&mut contents)?;
    let auth_dot_json: AuthDotJson = serde_json::from_str(&contents)?;
    Ok(auth_dot_json)
}

fn write_auth_json(auth_file: &Path, auth_dot_json: &AuthDotJson) -> std::io::Result<()> {
    let json_data = serde_json::to_string_pretty(auth_dot_json)?;
    let mut options = OpenOptions::new();
    options.truncate(true).write(true).create(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    let mut file = options.open(auth_file)?;
    use std::io::Write as _;
    file.write_all(json_data.as_bytes())?;
    file.flush()?;
    Ok(())
}

pub fn login_with_api_key(codex_home: &Path, api_key: &str) -> std::io::Result<()> {
    let auth_dot_json = AuthDotJson {
        openai_api_key: Some(api_key.to_string()),
        tokens: None,
        last_refresh: None,
    };
    write_auth_json(&get_auth_file(codex_home), &auth_dot_json)
}

pub(crate) fn update_tokens(
    auth_file: &Path,
    id_token: String,
    access_token: Option<String>,
    refresh_token: Option<String>,
) -> std::io::Result<AuthDotJson> {
    // Read, modify, write raw JSON to preserve id_token as a string on disk.
    let mut contents = String::new();
    {
        let mut f = File::open(auth_file)?;
        use std::io::Read as _;
        f.read_to_string(&mut contents)?;
    }
    let mut obj: serde_json::Value = serde_json::from_str(&contents)?;
    obj["tokens"]["id_token"] = serde_json::Value::String(id_token);
    if let Some(a) = access_token {
        obj["tokens"]["access_token"] = serde_json::Value::String(a);
    }
    if let Some(r) = refresh_token {
        obj["tokens"]["refresh_token"] = serde_json::Value::String(r);
    }
    obj["last_refresh"] =
        serde_json::Value::String(Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true));
    let updated = serde_json::to_string_pretty(&obj)?;
    std::fs::write(auth_file, updated)?;
    // Return parsed structure
    try_read_auth_json(auth_file)
}

/// Write a fresh auth.json to `codex_home` with the provided values.
/// Creates the directory if needed.
pub(crate) fn write_new_auth_json(
    codex_home: &Path,
    api_key: Option<String>,
    id_token: &str,
    access_token: &str,
    refresh_token: &str,
    account_id: Option<String>,
) -> std::io::Result<()> {
    std::fs::create_dir_all(codex_home)?;
    let auth_file = get_auth_file(codex_home);
    // Write explicit JSON preserving raw JWT in id_token
    let json_data = serde_json::json!({
        "OPENAI_API_KEY": api_key,
        "tokens": {
            "id_token": id_token,
            "access_token": access_token,
            "refresh_token": refresh_token,
            "account_id": account_id,
        },
        "last_refresh": Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
    });
    let contents = serde_json::to_string_pretty(&json_data)?;
    let mut options = OpenOptions::new();
    options.truncate(true).write(true).create(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    let mut file = options.open(&auth_file)?;
    use std::io::Write as _;
    file.write_all(contents.as_bytes())?;
    file.flush()
}
