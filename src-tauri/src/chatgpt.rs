//! ChatGPT plan usage: browser sign-in, token refresh, sign-out, and the
//! Responses API vision transport.
//!
//! Only the small session record (client id, host id, e-mail, refresh token)
//! is stored in the credential store; access tokens stay in memory.
use std::io::{BufRead, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine;
use jsonwebtoken::jwk::JwkSet;
use serde::Serialize;
use serde_json::{json, Value};

use crate::credentials::CredentialStore;
use crate::domain::{AppError, AppResult, VisionModel};
use crate::oauth::{self, RefreshFailure, StoredSession, TokenResponse};
use crate::providers::{bounded_body, Transport, VisionRequest, VisionResponse};

pub const PROVIDER_ID: &str = "chatgpt";
pub const REDIRECT_PORT: u16 = 47836;
const SESSION_KEY: &str = "chatgpt-session";
const API_BASE: &str = "https://api.openai.com/v1";
const APP_NAME: &str = "MetaPic Interrogator";
const INSTRUCTIONS: &str = "You describe images accurately and concisely.";
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);
const REFRESH_MARGIN: Duration = Duration::from_secs(60);
const MAX_AUTH_BODY_BYTES: usize = 256 * 1024;
const MAX_STREAM_BYTES: usize = 4 * 1024 * 1024;

struct Access {
    token: String,
    expires_at: Instant,
}

static ACCESS: Mutex<Option<Access>> = Mutex::new(None);
static REFRESH_LOCK: Mutex<()> = Mutex::new(());
static CANCEL: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChatGptStatus {
    pub configured: bool,
    pub email: Option<String>,
}

pub(crate) fn auth_client() -> AppResult<reqwest::blocking::Client> {
    crate::network::client_builder()?
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| AppError::Network("ChatGPT client unavailable".to_owned()))
}

pub(crate) fn post_form(
    client: &reqwest::blocking::Client,
    url: &str,
    form: &[(&'static str, String)],
) -> AppResult<(u16, String)> {
    let response = client
        .post(url)
        .form(form)
        .send()
        .map_err(|_| AppError::Network("ChatGPT sign-in service unreachable".to_owned()))?;
    let status = response.status().as_u16();
    let mut bytes = Vec::new();
    response
        .take((MAX_AUTH_BODY_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| AppError::Network("ChatGPT sign-in response unreadable".to_owned()))?;
    Ok((status, bounded_body(&bytes, MAX_AUTH_BODY_BYTES)?))
}

fn load_session(store: &dyn CredentialStore) -> AppResult<Option<StoredSession>> {
    Ok(store
        .get(SESSION_KEY)?
        .and_then(|raw| serde_json::from_str(&raw).ok()))
}

fn save_session(store: &mut dyn CredentialStore, session: &StoredSession) -> AppResult<()> {
    let raw = serde_json::to_string(session)
        .map_err(|_| AppError::Authentication("ChatGPT session could not be saved".to_owned()))?;
    store.set(SESSION_KEY, &raw)
}

pub fn status(store: &dyn CredentialStore) -> AppResult<ChatGptStatus> {
    Ok(match load_session(store)? {
        Some(session) => ChatGptStatus {
            configured: true,
            email: Some(session.email).filter(|email| !email.is_empty()),
        },
        None => ChatGptStatus {
            configured: false,
            email: None,
        },
    })
}

fn cache_access(reply: &TokenResponse) {
    if let Ok(mut guard) = ACCESS.lock() {
        *guard = Some(Access {
            token: reply.access_token.clone(),
            expires_at: Instant::now() + Duration::from_secs(reply.expires_in),
        });
    }
}

fn clear_access() {
    if let Ok(mut guard) = ACCESS.lock() {
        *guard = None;
    }
}

fn cached_token() -> Option<String> {
    let guard = ACCESS.lock().ok()?;
    let access = guard.as_ref()?;
    (access.expires_at > Instant::now() + REFRESH_MARGIN).then(|| access.token.clone())
}

/// Returns a valid access token, refreshing with the stored refresh token
/// when needed. Refreshes are serialized because refresh tokens rotate.
pub fn access_token(store: &mut dyn CredentialStore) -> AppResult<String> {
    if let Some(token) = cached_token() {
        return Ok(token);
    }
    let _guard = REFRESH_LOCK
        .lock()
        .map_err(|_| AppError::Authentication("ChatGPT refresh unavailable".to_owned()))?;
    if let Some(token) = cached_token() {
        return Ok(token);
    }
    let mut session = load_session(store)?.ok_or_else(|| {
        AppError::Authentication("sign in with ChatGPT in Settings first".to_owned())
    })?;
    let client = auth_client()?;
    let (status, body) = post_form(
        &client,
        oauth::TOKEN_URL,
        &oauth::refresh_form(&session.client_id, &session.refresh_token),
    )?;
    if !(200..300).contains(&status) {
        return Err(match oauth::classify_refresh_failure(status, &body) {
            RefreshFailure::ReauthRequired => {
                clear_access();
                store.delete(SESSION_KEY)?;
                AppError::Authentication(
                    "ChatGPT sign-in expired or was revoked; sign in again".to_owned(),
                )
            }
            RefreshFailure::InvalidClient => {
                AppError::Authentication("ChatGPT client registration is invalid".to_owned())
            }
            RefreshFailure::Transient => AppError::Network(format!(
                "ChatGPT token refresh failed (HTTP {status}: {}); try again",
                oauth::error_summary(&body)
            )),
        });
    }
    let reply = TokenResponse::parse(&body)?;
    cache_access(&reply);
    if let Some(rotated) = reply
        .refresh_token
        .as_deref()
        .filter(|token| !token.is_empty() && *token != session.refresh_token)
    {
        session.refresh_token = rotated.to_owned();
        save_session(store, &session)?;
    }
    Ok(reply.access_token)
}

/// Stops a sign-in that is waiting for the browser.
pub fn cancel_sign_in() {
    CANCEL.store(true, Ordering::Release);
}

/// Runs the browser sign-in. `open_browser` receives the authorization URL.
pub fn sign_in(
    store: &mut dyn CredentialStore,
    open_browser: impl FnOnce(&str) -> AppResult<()>,
) -> AppResult<ChatGptStatus> {
    CANCEL.store(false, Ordering::Release);
    let client = auth_client()?;
    let previous = load_session(store)?;
    let host_id = match &previous {
        Some(session) => session.host_id.clone(),
        None => oauth::new_host_id()?,
    };
    let listener = TcpListener::bind(("127.0.0.1", REDIRECT_PORT)).map_err(|_| {
        AppError::Network(format!(
            "port {REDIRECT_PORT} is busy; finish the other sign-in and try again"
        ))
    })?;
    let redirect = oauth::redirect_uri(REDIRECT_PORT);
    let attempt = oauth::AuthAttempt::new()?;
    let url = oauth::authorize_url(
        previous.as_ref().map(|session| session.client_id.as_str()),
        APP_NAME,
        &host_id,
        &redirect,
        &attempt,
    )?;
    open_browser(&url)?;

    let target = wait_for_callback(&listener)?;
    let callback = oauth::parse_callback(&target, &attempt.state)?;
    let client_id =
        oauth::resolve_client_id(previous.as_ref(), callback.issued_client_id.as_deref())?;

    let (http_status, body) = post_form(
        &client,
        oauth::TOKEN_URL,
        &oauth::exchange_form(&client_id, &callback.code, &attempt.verifier, &redirect),
    )?;
    if !(200..300).contains(&http_status) {
        return Err(AppError::Authentication(format!(
            "ChatGPT sign-in was rejected (HTTP {http_status}: {})",
            oauth::error_summary(&body)
        )));
    }
    let reply = TokenResponse::parse(&body)?;
    reply.require_plan_scope()?;
    let refresh_token = reply
        .refresh_token
        .clone()
        .filter(|token| !token.is_empty())
        .ok_or_else(|| {
            AppError::Authentication("ChatGPT did not return a refresh token".to_owned())
        })?;
    let id_token = reply.id_token.as_deref().ok_or_else(|| {
        AppError::Authentication("ChatGPT did not return an identity token".to_owned())
    })?;
    let jwks: JwkSet = client
        .get(oauth::JWKS_URL)
        .send()
        .and_then(|response| response.json())
        .map_err(|_| AppError::Network("ChatGPT signing keys unavailable".to_owned()))?;
    let identity = oauth::validate_id_token(id_token, &jwks, &client_id, Some(&attempt.nonce))?;
    cache_access(&reply);
    save_session(
        store,
        &StoredSession {
            client_id,
            host_id,
            email: identity.email,
            refresh_token,
        },
    )?;
    status(store)
}

/// Revokes the refresh token (best effort) and clears local data.
pub fn sign_out(store: &mut dyn CredentialStore) -> AppResult<()> {
    if let Ok(Some(session)) = load_session(store) {
        if let Ok(client) = auth_client() {
            let _ = post_form(
                &client,
                oauth::REVOKE_URL,
                &oauth::revoke_form(&session.client_id, &session.refresh_token),
            );
        }
    }
    clear_access();
    store.delete(SESSION_KEY)
}

fn wait_for_callback(listener: &TcpListener) -> AppResult<String> {
    listener
        .set_nonblocking(true)
        .map_err(|_| AppError::Network("could not listen for the browser".to_owned()))?;
    let deadline = Instant::now() + SIGN_IN_TIMEOUT;
    loop {
        if CANCEL.load(Ordering::Acquire) {
            return Err(AppError::Cancellation(
                "ChatGPT sign-in cancelled".to_owned(),
            ));
        }
        if Instant::now() > deadline {
            return Err(AppError::Network(
                "ChatGPT sign-in timed out; try again".to_owned(),
            ));
        }
        match listener.accept() {
            Ok((stream, _)) => {
                if let Some(target) = handle_connection(stream) {
                    return Ok(target);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => return Err(AppError::Network("browser connection failed".to_owned())),
        }
    }
}

fn handle_connection(mut stream: TcpStream) -> Option<String> {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buffer = [0_u8; 8192];
    let count = stream.read(&mut buffer).ok()?;
    let target = oauth::callback_target(&String::from_utf8_lossy(&buffer[..count]), REDIRECT_PORT);
    let (status, body) = match target {
        Some(_) => ("200 OK", "You can close this tab and return to MetaPic."),
        None => ("404 Not Found", "Not found."),
    };
    let page = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>MetaPic</title><body style=\"font-family:sans-serif;padding:2rem\"><p>{body}</p>"
    );
    let _ = stream.write_all(
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{page}",
            page.len()
        )
        .as_bytes(),
    );
    target
}

/// Opens an HTTPS URL in the default browser without going through a shell.
pub fn open_in_browser(url: &str) -> AppResult<()> {
    if !url.starts_with("https://") {
        return Err(AppError::InvalidPath(
            "refusing to open a non-HTTPS URL".to_owned(),
        ));
    }
    #[cfg(windows)]
    {
        std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn()
            .map(|_| ())
            .map_err(|_| AppError::Network("could not open the browser".to_owned()))
    }
    #[cfg(not(windows))]
    {
        Err(AppError::NotImplemented(
            "opening the browser is supported on Windows only".to_owned(),
        ))
    }
}

pub fn build_responses_payload(model: &str, prompt: &str, mime: &str, encoded: &str) -> Value {
    // The preview requires `store: false` and `stream: true` and rejects
    // temperature, max_output_tokens and several other fields.
    json!({
        "model": model,
        "instructions": INSTRUCTIONS,
        "input": [{
            "role": "user",
            "content": [
                {"type": "input_text", "text": prompt},
                {"type": "input_image", "image_url": format!("data:{mime};base64,{encoded}")}
            ]
        }],
        "store": false,
        "stream": true,
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum StreamEvent {
    Delta(String),
    Completed(Option<u64>),
    Ignore,
}

fn failure(value: &Value) -> AppError {
    let code = value
        .pointer("/response/error/code")
        .or_else(|| value.pointer("/error/code"))
        .or_else(|| value.get("code"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if code.contains("usage_limit") {
        return AppError::RateLimit("ChatGPT plan usage limit reached".to_owned());
    }
    let message = value
        .pointer("/response/error/message")
        .or_else(|| value.pointer("/error/message"))
        .or_else(|| value.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("request failed");
    let cleaned: String = message
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(200)
        .collect();
    AppError::Network(format!("ChatGPT: {}", cleaned.trim()))
}

pub fn parse_stream_event(line: &str) -> AppResult<StreamEvent> {
    let Some(data) = line.strip_prefix("data:") else {
        return Ok(StreamEvent::Ignore);
    };
    let data = data.trim();
    if data.is_empty() || data == "[DONE]" {
        return Ok(StreamEvent::Ignore);
    }
    let value: Value = serde_json::from_str(data)
        .map_err(|_| AppError::MalformedResponse("ChatGPT stream event was not JSON".to_owned()))?;
    match value.get("type").and_then(Value::as_str) {
        Some("response.output_text.delta") => Ok(value
            .get("delta")
            .and_then(Value::as_str)
            .map_or(StreamEvent::Ignore, |delta| {
                StreamEvent::Delta(delta.to_owned())
            })),
        Some("response.completed") => Ok(StreamEvent::Completed(
            value
                .pointer("/response/usage/total_tokens")
                .and_then(Value::as_u64),
        )),
        Some("response.failed" | "response.incomplete" | "error") => Err(failure(&value)),
        _ => Ok(StreamEvent::Ignore),
    }
}

/// Reads a Responses SSE stream. It must end with `response.completed`.
pub fn read_stream<R: BufRead>(
    mut reader: R,
    model: &str,
    on_delta: &mut dyn FnMut(String),
) -> AppResult<VisionResponse> {
    let mut line = String::new();
    let mut total = 0usize;
    let mut text = String::new();
    let mut usage = None;
    let mut completed = false;
    loop {
        line.clear();
        let read = reader
            .read_line(&mut line)
            .map_err(|_| AppError::Network("ChatGPT stream was interrupted".to_owned()))?;
        if read == 0 {
            break;
        }
        total += read;
        if total > MAX_STREAM_BYTES {
            return Err(AppError::MalformedResponse(
                "ChatGPT response exceeded size limit".to_owned(),
            ));
        }
        match parse_stream_event(line.trim_end())? {
            StreamEvent::Delta(delta) => {
                text.push_str(&delta);
                on_delta(delta);
            }
            StreamEvent::Completed(tokens) => {
                usage = tokens;
                completed = true;
                break;
            }
            StreamEvent::Ignore => {}
        }
    }
    if !completed {
        return Err(AppError::Network(
            "ChatGPT stream ended before the response was complete".to_owned(),
        ));
    }
    if text.trim().is_empty() {
        return Err(AppError::MalformedResponse(
            "ChatGPT response had no text".to_owned(),
        ));
    }
    Ok(VisionResponse {
        text,
        model: model.to_owned(),
        usage,
    })
}

fn map_status(status: u16) -> AppError {
    match status {
        401 | 403 => AppError::Authentication("ChatGPT rejected the sign-in".to_owned()),
        429 => AppError::RateLimit("ChatGPT rate limit".to_owned()),
        500..=599 => AppError::Network("ChatGPT server error".to_owned()),
        _ => AppError::MalformedResponse(format!("ChatGPT returned status {status}")),
    }
}

pub struct ResponsesTransport {
    client: reqwest::blocking::Client,
    base: String,
    access_token: String,
    model: String,
}

impl ResponsesTransport {
    pub fn new(access_token: &str, model: &str) -> AppResult<Self> {
        Self::with_base(API_BASE, access_token, model)
    }

    pub fn with_base(base: &str, access_token: &str, model: &str) -> AppResult<Self> {
        if access_token.is_empty() || model.trim().is_empty() {
            return Err(AppError::InvalidPath(
                "ChatGPT sign-in and model are required".to_owned(),
            ));
        }
        let client = crate::network::client_builder()?
            .timeout(Duration::from_secs(180))
            .build()
            .map_err(|_| AppError::Network("ChatGPT client unavailable".to_owned()))?;
        Ok(Self {
            client,
            base: base.trim_end_matches('/').to_owned(),
            access_token: access_token.to_owned(),
            model: model.to_owned(),
        })
    }
}

impl Transport for ResponsesTransport {
    fn request(&self, provider: &str, request: &VisionRequest) -> AppResult<VisionResponse> {
        self.stream_request(provider, request, &mut |_| {})
    }

    fn stream_request(
        &self,
        _provider: &str,
        request: &VisionRequest,
        on_delta: &mut dyn FnMut(String),
    ) -> AppResult<VisionResponse> {
        let encoded = base64::engine::general_purpose::STANDARD.encode(&request.image);
        let payload =
            build_responses_payload(&self.model, &request.prompt, &request.mime, &encoded);
        let response = self
            .client
            .post(format!("{}/responses", self.base))
            .bearer_auth(&self.access_token)
            .header("Accept", "text/event-stream")
            .json(&payload)
            .send()
            .map_err(|_| AppError::Network("ChatGPT request failed".to_owned()))?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(map_status(status));
        }
        read_stream(std::io::BufReader::new(response), &self.model, on_delta)
    }
}

pub fn parse_models(value: &Value) -> AppResult<Vec<VisionModel>> {
    if let Some(data) = value.get("data").and_then(Value::as_array) {
        return Ok(data
            .iter()
            .filter_map(|model| model.get("id").and_then(Value::as_str))
            .filter(|id| !id.is_empty() && id.len() <= 200)
            .map(|id| VisionModel {
                id: id.to_owned(),
                display_name: id.to_owned(),
                vision_capable: true,
            })
            .collect());
    }
    let models = value
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            AppError::MalformedResponse("ChatGPT model list has an unexpected shape".to_owned())
        })?;
    Ok(models
        .iter()
        .filter(|model| {
            model
                .get("visibility")
                .and_then(Value::as_str)
                .is_none_or(|visibility| visibility == "list")
        })
        .filter_map(|model| {
            let slug = model.get("slug").and_then(Value::as_str)?;
            (!slug.is_empty() && slug.len() <= 200).then(|| VisionModel {
                id: slug.to_owned(),
                display_name: model
                    .get("display_name")
                    .and_then(Value::as_str)
                    .unwrap_or(slug)
                    .chars()
                    .take(120)
                    .collect(),
                vision_capable: true,
            })
        })
        .collect())
}

pub fn list_models(access_token: &str) -> AppResult<Vec<VisionModel>> {
    let models = fetch_models(API_BASE, access_token)?;
    if models.is_empty() {
        return Err(AppError::UnavailableModel(
            "ChatGPT returned no usable models for this plan".to_owned(),
        ));
    }
    Ok(models)
}

pub(crate) fn fetch_models(base: &str, access_token: &str) -> AppResult<Vec<VisionModel>> {
    let response = auth_client()?
        .get(format!("{base}/models"))
        .bearer_auth(access_token)
        .send()
        .map_err(|_| AppError::Network("ChatGPT model list unavailable".to_owned()))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(match map_status(status) {
            AppError::MalformedResponse(_) => {
                AppError::MalformedResponse(format!("ChatGPT model list returned status {status}"))
            }
            other => other,
        });
    }
    let value: Value = response
        .json()
        .map_err(|_| AppError::MalformedResponse("ChatGPT model list was not JSON".to_owned()))?;
    parse_models(&value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::MemoryCredentialStore;

    #[test]
    fn payload_follows_plan_usage_rules_and_carries_the_image() {
        let payload = build_responses_payload("gpt-x", "Describe", "image/png", "QUJD");
        assert_eq!(payload["store"], false);
        assert_eq!(payload["stream"], true);
        for field in [
            "temperature",
            "max_output_tokens",
            "previous_response_id",
            "user",
        ] {
            assert!(payload.get(field).is_none(), "{field}");
        }
        assert_eq!(payload["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(
            payload["input"][0]["content"][1]["image_url"],
            "data:image/png;base64,QUJD"
        );
    }

    #[test]
    fn stream_collects_text_and_requires_completion() {
        let stream = "event: response.created\ndata: {\"type\":\"response.created\"}\n\n\
data: {\"type\":\"response.output_text.delta\",\"delta\":\"A \"}\n\n\
data: {\"type\":\"response.output_text.delta\",\"delta\":\"cat.\"}\n\n\
data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"total_tokens\":9}}}\n\n";
        let mut seen = Vec::new();
        let response = read_stream(stream.as_bytes(), "m", &mut |d| seen.push(d)).expect("ok");
        assert_eq!(response.text, "A cat.");
        assert_eq!(response.usage, Some(9));
        assert_eq!(seen, ["A ", "cat."]);

        let truncated = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"x\"}\n\n";
        assert!(read_stream(truncated.as_bytes(), "m", &mut |_| {}).is_err());
    }

    #[test]
    fn usage_limit_maps_to_rate_limit_and_other_failures_are_sanitized() {
        let limit = "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"subscription_sharing_usage_limit_exceeded\"}}}\n";
        assert!(matches!(
            parse_stream_event(limit),
            Err(AppError::RateLimit(_))
        ));
        let other = "data: {\"type\":\"error\",\"message\":\"bad\\nthing\"}\n";
        match parse_stream_event(other) {
            Err(AppError::Network(text)) => assert_eq!(text, "ChatGPT: bad thing"),
            result => panic!("unexpected {result:?}"),
        }
        assert_eq!(
            parse_stream_event("event: ping").expect("ignored"),
            StreamEvent::Ignore
        );
    }

    #[test]
    fn only_listed_models_are_offered() {
        let models = parse_models(&json!({"models":[
            {"slug":"gpt-a","display_name":"GPT A","visibility":"list"},
            {"slug":"hidden","visibility":"hide"}
        ]}))
        .expect("models");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "gpt-a");
        assert!(parse_models(&json!({})).is_err());
        let openai_style =
            parse_models(&json!({"data":[{"id":"gpt-b"},{"id":""}]})).expect("data shape");
        assert_eq!(openai_style.len(), 1);
        assert_eq!(openai_style[0].id, "gpt-b");
    }

    #[test]
    fn status_reflects_stored_session_without_exposing_tokens() {
        let mut store = MemoryCredentialStore::default();
        assert!(!status(&store).expect("status").configured);
        save_session(
            &mut store,
            &StoredSession {
                client_id: "c".to_owned(),
                host_id: "h".to_owned(),
                email: "me@example.com".to_owned(),
                refresh_token: "r".to_owned(),
            },
        )
        .expect("save");
        let found = status(&store).expect("status");
        assert!(found.configured);
        assert_eq!(found.email.as_deref(), Some("me@example.com"));
    }

    #[test]
    fn browser_opener_rejects_non_https() {
        assert!(open_in_browser("file:///c:/windows/system32/calc.exe").is_err());
        assert!(open_in_browser("http://example.com").is_err());
    }
}
