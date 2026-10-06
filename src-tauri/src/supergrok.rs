//! SuperGrok subscription sign-in for xAI.
//!
//! OAuth 2.0 device code flow (RFC 8628) against `auth.x.ai` as a public
//! client with xAI's shared Grok client id; the access token is used as a
//! bearer token for `api.x.ai/v1`. xAI decides which accounts receive tokens.
//!
//! Only `{refresh_token, email}` is stored in the credential store; access
//! tokens stay in memory and are refreshed on demand.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::chatgpt::{fetch_models, post_form};
use crate::credentials::CredentialStore;
use crate::domain::{AppError, AppResult, VisionModel};
use crate::oauth::{self, RefreshFailure, TokenResponse};

pub const PROVIDER_ID: &str = "xai";
pub const API_BASE: &str = "https://api.x.ai/v1";
const SESSION_KEY: &str = "supergrok-session";
const ISSUER: &str = "https://auth.x.ai";
const CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
const SCOPES: &str = "openid profile email offline_access grok-cli:access api:access";
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
const REFRESH_MARGIN: Duration = Duration::from_secs(120);
const MIN_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Session {
    refresh_token: String,
    email: String,
}

#[derive(Clone)]
struct Endpoints {
    device: String,
    token: String,
    revocation: Option<String>,
}

struct Access {
    token: String,
    expires_at: Instant,
}

struct Pending {
    device_code: String,
    interval: Duration,
    expires_at: Instant,
}

/// What the user must do in the browser to approve the sign-in.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCode {
    pub user_code: String,
    pub verification_url: String,
    pub expires_in: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SuperGrokStatus {
    pub configured: bool,
    pub email: Option<String>,
}

static ACCESS: Mutex<Option<Access>> = Mutex::new(None);
static ENDPOINTS: Mutex<Option<Endpoints>> = Mutex::new(None);
static PENDING: Mutex<Option<Pending>> = Mutex::new(None);
static REFRESH_LOCK: Mutex<()> = Mutex::new(());
static CANCEL: AtomicBool = AtomicBool::new(false);

fn parse_endpoints(value: &Value) -> AppResult<Endpoints> {
    let field = |name: &str| {
        value
            .get(name)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| {
                AppError::MalformedResponse(format!("xAI OpenID configuration lacks {name}"))
            })
    };
    Ok(Endpoints {
        device: field("device_authorization_endpoint")?,
        token: field("token_endpoint")?,
        revocation: field("revocation_endpoint").ok(),
    })
}

fn endpoints(client: &reqwest::blocking::Client) -> AppResult<Endpoints> {
    if let Some(cached) = ENDPOINTS.lock().ok().and_then(|guard| guard.clone()) {
        return Ok(cached);
    }
    let value: Value = client
        .get(format!("{ISSUER}/.well-known/openid-configuration"))
        .send()
        .and_then(|response| response.json())
        .map_err(|_| AppError::Network("xAI sign-in service unreachable".to_owned()))?;
    let found = parse_endpoints(&value)?;
    if let Ok(mut guard) = ENDPOINTS.lock() {
        *guard = Some(found.clone());
    }
    Ok(found)
}

fn load_session(store: &dyn CredentialStore) -> AppResult<Option<Session>> {
    Ok(store
        .get(SESSION_KEY)?
        .and_then(|raw| serde_json::from_str(&raw).ok()))
}

fn save_session(store: &mut dyn CredentialStore, session: &Session) -> AppResult<()> {
    let raw = serde_json::to_string(session)
        .map_err(|_| AppError::Authentication("SuperGrok session could not be saved".to_owned()))?;
    store.set(SESSION_KEY, &raw)
}

pub fn status(store: &dyn CredentialStore) -> AppResult<SuperGrokStatus> {
    Ok(match load_session(store)? {
        Some(session) => SuperGrokStatus {
            configured: true,
            email: Some(session.email).filter(|email| !email.is_empty()),
        },
        None => SuperGrokStatus {
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
        .map_err(|_| AppError::Authentication("SuperGrok refresh unavailable".to_owned()))?;
    if let Some(token) = cached_token() {
        return Ok(token);
    }
    let mut session = load_session(store)?.ok_or_else(|| {
        AppError::Authentication("sign in with SuperGrok in Settings first".to_owned())
    })?;
    let client = crate::chatgpt::auth_client()?;
    let endpoints = endpoints(&client)?;
    let (status, body) = post_form(
        &client,
        &endpoints.token,
        &[
            ("grant_type", "refresh_token".to_owned()),
            ("client_id", CLIENT_ID.to_owned()),
            ("refresh_token", session.refresh_token.clone()),
        ],
    )?;
    if !(200..300).contains(&status) {
        return Err(match oauth::classify_refresh_failure(status, &body) {
            RefreshFailure::ReauthRequired => {
                clear_access();
                store.delete(SESSION_KEY)?;
                AppError::Authentication(
                    "SuperGrok sign-in expired or was revoked; sign in again".to_owned(),
                )
            }
            RefreshFailure::InvalidClient => {
                AppError::Authentication("SuperGrok client registration is invalid".to_owned())
            }
            RefreshFailure::Transient => AppError::Network(format!(
                "SuperGrok token refresh failed (HTTP {status}: {}); try again",
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

fn parse_device_reply(reply: &Value) -> AppResult<(Pending, DeviceCode)> {
    let text = |name: &str| {
        reply
            .get(name)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| AppError::MalformedResponse(format!("xAI device response lacks {name}")))
    };
    let device_code = text("device_code")?;
    let user_code = text("user_code")?;
    let verification_url =
        text("verification_uri_complete").or_else(|_| text("verification_uri"))?;
    let expires_in = reply
        .get("expires_in")
        .and_then(Value::as_u64)
        .unwrap_or(600);
    let interval = Duration::from_secs(reply.get("interval").and_then(Value::as_u64).unwrap_or(5))
        .max(MIN_INTERVAL);
    Ok((
        Pending {
            device_code,
            interval,
            expires_at: Instant::now() + Duration::from_secs(expires_in),
        },
        DeviceCode {
            user_code,
            verification_url,
            expires_in,
        },
    ))
}

/// Requests a device code. The caller opens `verification_url` and shows
/// `user_code`, then calls [`finish_sign_in`].
pub fn begin_sign_in() -> AppResult<DeviceCode> {
    CANCEL.store(false, Ordering::Release);
    let client = crate::chatgpt::auth_client()?;
    let endpoints = endpoints(&client)?;
    let (status, body) = post_form(
        &client,
        &endpoints.device,
        &[
            ("client_id", CLIENT_ID.to_owned()),
            ("scope", SCOPES.to_owned()),
        ],
    )?;
    if !(200..300).contains(&status) {
        return Err(AppError::Authentication(format!(
            "xAI refused the sign-in request (HTTP {status}: {})",
            oauth::error_summary(&body)
        )));
    }
    let reply: Value = serde_json::from_str(&body)
        .map_err(|_| AppError::MalformedResponse("xAI device response was not JSON".to_owned()))?;
    let (pending, code) = parse_device_reply(&reply)?;
    if let Ok(mut guard) = PENDING.lock() {
        *guard = Some(pending);
    }
    Ok(code)
}

enum Poll {
    Waiting,
    SlowDown,
    Done(Box<TokenResponse>),
    Failed(AppError),
}

fn poll_outcome(status: u16, body: &str) -> Poll {
    if (200..300).contains(&status) {
        return match TokenResponse::parse(body) {
            Ok(reply) => Poll::Done(Box::new(reply)),
            Err(error) => Poll::Failed(error),
        };
    }
    match oauth::error_code(body).as_str() {
        "authorization_pending" => Poll::Waiting,
        "slow_down" => Poll::SlowDown,
        "access_denied" => Poll::Failed(AppError::Authentication(
            "SuperGrok sign-in was declined".to_owned(),
        )),
        "expired_token" => Poll::Failed(AppError::Network(
            "SuperGrok sign-in expired; try again".to_owned(),
        )),
        _ => Poll::Failed(AppError::Authentication(format!(
            "SuperGrok sign-in failed (HTTP {status}: {})",
            oauth::error_summary(body)
        ))),
    }
}

/// Polls until the user approves (or refuses, or the code expires) and stores
/// the session.
pub fn finish_sign_in(store: &mut dyn CredentialStore) -> AppResult<SuperGrokStatus> {
    let Pending {
        device_code,
        mut interval,
        expires_at,
    } = PENDING
        .lock()
        .ok()
        .and_then(|mut guard| guard.take())
        .ok_or_else(|| AppError::InvalidPath("no SuperGrok sign-in is in progress".to_owned()))?;
    let client = crate::chatgpt::auth_client()?;
    let endpoints = endpoints(&client)?;
    loop {
        let wake = Instant::now() + interval;
        while Instant::now() < wake {
            if CANCEL.load(Ordering::Acquire) {
                return Err(AppError::Cancellation(
                    "SuperGrok sign-in cancelled".to_owned(),
                ));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if Instant::now() > expires_at {
            return Err(AppError::Network(
                "SuperGrok sign-in timed out; try again".to_owned(),
            ));
        }
        let (status, body) = post_form(
            &client,
            &endpoints.token,
            &[
                ("grant_type", DEVICE_GRANT.to_owned()),
                ("device_code", device_code.clone()),
                ("client_id", CLIENT_ID.to_owned()),
            ],
        )?;
        match poll_outcome(status, &body) {
            Poll::Waiting => {}
            Poll::SlowDown => interval += MIN_INTERVAL,
            Poll::Failed(error) => return Err(error),
            Poll::Done(reply) => return complete(store, &reply),
        }
    }
}

fn complete(store: &mut dyn CredentialStore, reply: &TokenResponse) -> AppResult<SuperGrokStatus> {
    let refresh_token = reply
        .refresh_token
        .clone()
        .filter(|token| !token.is_empty())
        .ok_or_else(|| AppError::Authentication("xAI did not return a refresh token".to_owned()))?;
    cache_access(reply);
    let email = reply
        .id_token
        .as_deref()
        .and_then(email_from_id_token)
        .unwrap_or_default();
    save_session(
        store,
        &Session {
            refresh_token,
            email,
        },
    )?;
    status(store)
}

/// Reads the e-mail claim for display only; nothing is trusted from it.
fn email_from_id_token(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let claims: Value = serde_json::from_slice(&bytes).ok()?;
    claims
        .get("email")
        .or_else(|| claims.get("name"))
        .and_then(Value::as_str)
        .map(|value| value.chars().take(120).collect())
}

/// Stops a sign-in that is waiting for the browser.
pub fn cancel_sign_in() {
    CANCEL.store(true, Ordering::Release);
    if let Ok(mut guard) = PENDING.lock() {
        *guard = None;
    }
}

/// Revokes the refresh token (best effort) and clears local data.
pub fn sign_out(store: &mut dyn CredentialStore) -> AppResult<()> {
    if let Ok(Some(session)) = load_session(store) {
        if let Ok(client) = crate::chatgpt::auth_client() {
            if let Some(url) = endpoints(&client).ok().and_then(|found| found.revocation) {
                let _ = post_form(
                    &client,
                    &url,
                    &[
                        ("token", session.refresh_token),
                        ("token_type_hint", "refresh_token".to_owned()),
                        ("client_id", CLIENT_ID.to_owned()),
                    ],
                );
            }
        }
    }
    clear_access();
    store.delete(SESSION_KEY)
}

/// Text-generation models only: image, video and embedding models cannot
/// describe a picture.
pub fn describing_models(models: Vec<VisionModel>) -> Vec<VisionModel> {
    models
        .into_iter()
        .filter(|model| {
            let id = model.id.to_ascii_lowercase();
            !["image", "imagine", "video", "embed"]
                .iter()
                .any(|word| id.contains(word))
        })
        .collect()
}

pub fn list_models(access_token: &str) -> AppResult<Vec<VisionModel>> {
    let models = describing_models(fetch_models(API_BASE, access_token)?);
    if models.is_empty() {
        return Err(AppError::UnavailableModel(
            "xAI returned no usable models for this account".to_owned(),
        ));
    }
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::MemoryCredentialStore;
    use serde_json::json;

    #[test]
    fn endpoints_need_device_and_token_urls() {
        let found = parse_endpoints(&json!({
            "device_authorization_endpoint": "https://auth.x.ai/device",
            "token_endpoint": "https://auth.x.ai/token"
        }))
        .expect("endpoints");
        assert!(found.revocation.is_none());
        assert!(parse_endpoints(&json!({"token_endpoint": "x"})).is_err());
    }

    #[test]
    fn device_reply_prefers_complete_uri_and_enforces_min_interval() {
        let (pending, code) = parse_device_reply(&json!({
            "device_code": "dc",
            "user_code": "ABCD-1234",
            "verification_uri": "https://x.ai/device",
            "verification_uri_complete": "https://x.ai/device?code=ABCD-1234",
            "expires_in": 300,
            "interval": 1
        }))
        .expect("reply");
        assert_eq!(code.user_code, "ABCD-1234");
        assert_eq!(code.verification_url, "https://x.ai/device?code=ABCD-1234");
        assert_eq!(pending.interval, MIN_INTERVAL);
        assert!(parse_device_reply(&json!({"user_code": "x"})).is_err());
    }

    #[test]
    fn polling_follows_rfc8628() {
        let error = |code: &str| format!(r#"{{"error":"{code}"}}"#);
        assert!(matches!(
            poll_outcome(400, &error("authorization_pending")),
            Poll::Waiting
        ));
        assert!(matches!(
            poll_outcome(400, &error("slow_down")),
            Poll::SlowDown
        ));
        assert!(matches!(
            poll_outcome(400, &error("access_denied")),
            Poll::Failed(AppError::Authentication(_))
        ));
        assert!(matches!(
            poll_outcome(400, &error("expired_token")),
            Poll::Failed(AppError::Network(_))
        ));
        assert!(matches!(
            poll_outcome(200, r#"{"access_token":"a","refresh_token":"r"}"#),
            Poll::Done(_)
        ));
        assert!(matches!(poll_outcome(200, "nope"), Poll::Failed(_)));
    }

    #[test]
    fn email_is_read_for_display_and_session_status_hides_tokens() {
        let payload = URL_SAFE_NO_PAD.encode(br#"{"email":"me@example.com"}"#);
        assert_eq!(
            email_from_id_token(&format!("e30.{payload}.sig")).as_deref(),
            Some("me@example.com")
        );
        assert!(email_from_id_token("garbage").is_none());

        let mut store = MemoryCredentialStore::default();
        assert!(!status(&store).expect("status").configured);
        save_session(
            &mut store,
            &Session {
                refresh_token: "SECRET".to_owned(),
                email: "me@example.com".to_owned(),
            },
        )
        .expect("save");
        let found = status(&store).expect("status");
        assert!(found.configured);
        assert!(!format!("{found:?}").contains("SECRET"));
    }

    #[test]
    fn finishing_without_a_started_sign_in_is_an_error() {
        if let Ok(mut guard) = PENDING.lock() {
            *guard = None;
        }
        let mut store = MemoryCredentialStore::default();
        assert!(matches!(
            finish_sign_in(&mut store),
            Err(AppError::InvalidPath(_))
        ));
    }

    #[test]
    fn non_describing_models_are_filtered_out() {
        let model = |id: &str| VisionModel {
            id: id.to_owned(),
            display_name: id.to_owned(),
            vision_capable: true,
        };
        let kept = describing_models(vec![
            model("grok-4"),
            model("grok-2-image"),
            model("grok-imagine-video"),
            model("embed-1"),
        ]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].id, "grok-4");
    }
}
