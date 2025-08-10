//
use rand::RngCore;
use reqwest::blocking::Client;
use serde_json::json;
use std::collections::HashMap;
use std::net::TcpListener;
use std::path::Path;
use std::path::PathBuf;
#[cfg(feature = "http-e2e-tests")]
use std::time::Duration;
use tiny_http::Header;
use tiny_http::Method;
use tiny_http::Response;
use tiny_http::Server;
use url::Url;
use url::form_urlencoded;

use crate::auth_store::write_new_auth_json;
use crate::pkce::generate_pkce;
use crate::success_url::build_success_url;
use crate::token_data::extract_login_context_from_tokens;

pub const DEFAULT_PORT: u16 = 1455;
pub const DEFAULT_ISSUER: &str = "https://auth.openai.com";

pub const LOGIN_SUCCESS_HTML: &str = include_str!("./success_page.html");

#[derive(Debug, Clone)]
pub struct LoginServerOptions {
    pub codex_home: PathBuf,
    pub client_id: String,
    pub issuer: String,
    pub port: u16,
    pub open_browser: bool,
    pub redeem_credits: bool,
    pub expose_state_endpoint: bool,
    /// timeout after x secs for e2e tests
    pub testing_timeout_secs: Option<u64>,
    pub verbose: bool,
    #[cfg(feature = "http-e2e-tests")]
    pub port_sender: Option<std::sync::mpsc::Sender<u16>>,
}

// Only default issuer supported for platform/api bases
const PLATFORM_BASE: &str = "https://platform.openai.com";
const API_BASE: &str = "https://api.openai.com";

fn default_url_base(port: u16) -> String {
    format!("http://localhost:{port}")
}

#[allow(dead_code)]
pub fn run_local_login_server(codex_home: &Path, client_id: &str) -> std::io::Result<()> {
    let opts = LoginServerOptions {
        codex_home: codex_home.to_path_buf(),
        client_id: client_id.to_string(),
        issuer: DEFAULT_ISSUER.to_string(),
        port: DEFAULT_PORT,
        open_browser: true,
        redeem_credits: true,
        expose_state_endpoint: false,
        testing_timeout_secs: None,
        verbose: false,
        #[cfg(feature = "http-e2e-tests")]
        port_sender: None,
    };
    run_local_login_server_with_options(opts)
}

pub fn run_local_login_server_with_options(mut opts: LoginServerOptions) -> std::io::Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", opts.port))
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    let actual_port = listener
        .local_addr()
        .map_err(|e| std::io::Error::other(e.to_string()))?
        .port();
    opts.port = actual_port;

    #[cfg(feature = "http-e2e-tests")]
    if let Some(tx) = &opts.port_sender {
        let _ = tx.send(actual_port);
    }

    let server =
        Server::from_listener(listener, None).map_err(|e| std::io::Error::other(e.to_string()))?;

    let issuer = opts.issuer.clone();
    let url_base = default_url_base(opts.port);

    let pkce = generate_pkce();
    let state = {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        hex::encode(bytes)
    };

    let redirect_uri = format!("{url_base}/auth/callback");
    let auth_url_str = format!("{issuer}/oauth/authorize");
    let mut auth_url =
        Url::parse(&auth_url_str).map_err(|e| std::io::Error::other(e.to_string()))?;
    auth_url
        .query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &opts.client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("scope", "openid profile email offline_access")
        .append_pair("code_challenge", &pkce.code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("id_token_add_organizations", "true")
        .append_pair("codex_cli_simplified_flow", "true")
        .append_pair("state", &state);

    eprintln!("Starting local login server on {url_base}");
    if opts.open_browser {
        let _ = webbrowser::open(auth_url.as_str());
    }
    eprintln!(
        ". If your browser did not open, navigate to this URL to authenticate: \n\n{auth_url}"
    );

    // If a testing timeout is configured, schedule an internal exit request so tests don't hang CI.
    #[cfg(feature = "http-e2e-tests")]
    if let Some(secs) = opts.testing_timeout_secs {
        let port = opts.port;
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(secs));
            let _ = reqwest::blocking::get(format!("http://127.0.0.1:{port}/__test/exit"));
        });
    }

    // Main request loop
    'outer: loop {
        let request = match server.recv() {
            Ok(r) => r,
            Err(e) => {
                return Err(std::io::Error::other(e.to_string()));
            }
        };

        let full = request.url().to_string();
        let (path, query) = match full.split_once('?') {
            Some((p, q)) => (p.to_string(), Some(q.to_string())),
            None => (full.clone(), None),
        };

        if opts.verbose {
            eprintln!("{} {}", request.method().as_str(), request.url());
        }

        match (request.method().clone(), path.as_str()) {
            (Method::Get, "/success") => {
                let mut resp = Response::from_string(LOGIN_SUCCESS_HTML).with_status_code(200);
                if let Ok(h) =
                    Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..])
                {
                    resp.add_header(h);
                }
                let _ = request.respond(resp);
                break 'outer;
            }
            (Method::Get, "/__test/exit") => {
                let _ = request.respond(Response::from_string("bye").with_status_code(200));
                break 'outer;
            }
            // Test-only helper to retrieve the current state, enabled via options.
            #[cfg(feature = "http-e2e-tests")]
            (Method::Get, "/__test/state") if opts.expose_state_endpoint => {
                let mut resp = Response::from_string(state.clone()).with_status_code(200);
                if let Ok(h) = Header::from_bytes(&b"Content-Type"[..], &b"text/plain"[..]) {
                    resp.add_header(h);
                }
                let _ = request.respond(resp);
            }
            (Method::Get, "/auth/callback") => {
                // Parse query params
                let params: HashMap<String, String> =
                    form_urlencoded::parse(query.as_deref().unwrap_or("").as_bytes())
                        .into_owned()
                        .collect();

                // Preserve explicit error messages for tests
                if params.get("state").map(|s| s.as_str()) != Some(state.as_str()) {
                    let _ = request.respond(
                        Response::from_string("State parameter mismatch").with_status_code(400),
                    );
                    continue;
                }
                let code_opt = params.get("code").map(|s| s.as_str());
                if code_opt.map(|s| s.is_empty()).unwrap_or(true) {
                    let _ = request.respond(
                        Response::from_string("Missing authorization code").with_status_code(400),
                    );
                    continue;
                }

                // Delegate to shared headless callback handler
                let http = DefaultHttp::default();
                match process_callback_headless(
                    &opts,
                    &state,
                    &state,
                    code_opt,
                    &pkce.code_verifier,
                    &http,
                ) {
                    Ok(outcome) => {
                        let mut resp = Response::empty(302);
                        if let Ok(h) =
                            Header::from_bytes(&b"Location"[..], outcome.success_url.as_str())
                        {
                            resp.add_header(h);
                        }
                        let _ = request.respond(resp);
                    }
                    Err(_) => {
                        let _ = request.respond(
                            Response::from_string("Token exchange failed").with_status_code(500),
                        );
                    }
                }
            }
            _ => {
                let _ = request
                    .respond(Response::from_string("Endpoint not supported").with_status_code(404));
            }
        }
    }

    Ok(())
}

// -------- Headless testing helpers (no HTTP server) --------

#[derive(Debug, Clone)]
pub struct HeadlessOutcome {
    pub success_url: String,
    pub api_key: Option<String>,
}

pub trait Http {
    fn post_form(&self, url: &str, form: &[(String, String)])
    -> std::io::Result<serde_json::Value>;
    fn post_json(&self, url: &str, body: &serde_json::Value) -> std::io::Result<serde_json::Value>;
}

pub struct DefaultHttp(Client);
impl Default for DefaultHttp {
    fn default() -> Self {
        Self(Client::new())
    }
}
impl Http for DefaultHttp {
    fn post_form(
        &self,
        url: &str,
        form: &[(String, String)],
    ) -> std::io::Result<serde_json::Value> {
        let resp = self
            .0
            .post(url)
            .form(
                &form
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.as_str()))
                    .collect::<Vec<_>>(),
            )
            .send()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        let val = resp
            .json::<serde_json::Value>()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        Ok(val)
    }
    fn post_json(&self, url: &str, body: &serde_json::Value) -> std::io::Result<serde_json::Value> {
        let resp = self
            .0
            .post(url)
            .json(body)
            .send()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        let val = resp
            .json::<serde_json::Value>()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        Ok(val)
    }
}

#[derive(serde::Deserialize)]
struct TokenExchange {
    id_token: String,
    access_token: String,
    refresh_token: String,
}

pub fn process_callback_headless(
    opts: &LoginServerOptions,
    expected_state: &str,
    incoming_state: &str,
    code_opt: Option<&str>,
    code_verifier: &str,
    http: &dyn Http,
) -> std::io::Result<HeadlessOutcome> {
    if incoming_state != expected_state {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "state mismatch",
        ));
    }
    let code = code_opt.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "missing authorization code",
        )
    })?;

    let token_endpoint = format!("{}/oauth/token", opts.issuer);
    let redirect_uri = format!("{}/auth/callback", default_url_base(opts.port));

    let form = vec![
        ("grant_type".to_string(), "authorization_code".to_string()),
        ("code".to_string(), code.to_string()),
        ("redirect_uri".to_string(), redirect_uri.clone()),
        ("client_id".to_string(), opts.client_id.clone()),
        ("code_verifier".to_string(), code_verifier.to_string()),
    ];
    let tokens_val = http.post_form(&token_endpoint, &form)?;
    let TokenExchange {
        id_token,
        access_token,
        refresh_token,
    } = serde_json::from_value(tokens_val)
        .map_err(|e| std::io::Error::other(format!("invalid token response: {e}")))?;
    if id_token.is_empty() || access_token.is_empty() || refresh_token.is_empty() {
        return Err(std::io::Error::other("token exchange failed"));
    }

    let (account_id, org_id, project_id, needs_setup, plan_type) =
        extract_login_context_from_tokens(&id_token, &access_token);

    let api_key = None;

    write_new_auth_json(
        &opts.codex_home,
        api_key.clone(),
        &id_token,
        &access_token,
        &refresh_token,
        account_id,
    )?;

    if opts.redeem_credits {
        let redeem_url = format!("{API_BASE}/v1/billing/redeem_credits");
        let _ = http.post_json(&redeem_url, &json!({"id_token": id_token}));
    }

    let base = default_url_base(opts.port);
    let platform_url = PLATFORM_BASE;
    let success_url = build_success_url(
        &base,
        Some(&id_token),
        org_id.as_deref(),
        project_id.as_deref(),
        plan_type.as_deref(),
        needs_setup,
        platform_url,
    )
    .map_err(|e| std::io::Error::other(e.to_string()))?;

    Ok(HeadlessOutcome {
        success_url: success_url.to_string(),
        api_key,
    })
}

//
