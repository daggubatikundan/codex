use serde::Deserialize;
use serde::Serialize;

#[derive(Serialize)]
struct RefreshRequest {
    client_id: &'static str,
    grant_type: &'static str,
    refresh_token: String,
    scope: &'static str,
}

#[derive(Deserialize, Clone)]
pub(crate) struct RefreshResponse {
    pub(crate) id_token: String,
    pub(crate) access_token: Option<String>,
    pub(crate) refresh_token: Option<String>,
}

pub(crate) async fn try_refresh_token(refresh_token: String) -> std::io::Result<RefreshResponse> {
    let refresh_request = RefreshRequest {
        client_id: crate::CLIENT_ID,
        grant_type: "refresh_token",
        refresh_token,
        scope: "openid profile email",
    };

    let client = reqwest::Client::new();
    let response = client
        .post("https://auth.openai.com/oauth/token")
        .header("Content-Type", "application/json")
        .json(&refresh_request)
        .send()
        .await
        .map_err(std::io::Error::other)?;

    if response.status().is_success() {
        let refresh_response = response
            .json::<RefreshResponse>()
            .await
            .map_err(std::io::Error::other)?;
        Ok(refresh_response)
    } else {
        Err(std::io::Error::other(format!(
            "Failed to refresh token: {}",
            response.status()
        )))
    }
}
