//! Sign in with ChatGPT (open-source plan usage) protocol core.
//!
//! Pure, network-free building blocks: PKCE, authorization URL, callback
//! parsing, token response and ID token validation, and refresh failure
//! classification. Endpoints and parameters follow
//! https://developers.openai.com/siwc/token-sharing-open-source
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use jsonwebtoken::{jwk::JwkSet, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domain::{AppError, AppResult};

pub const AUTHORIZE_URL: &str = "https://auth.openai.com/api/accounts/authorize";
pub const TOKEN_URL: &str = "https://auth.openai.com/api/accounts/oauth/token";
pub const REVOKE_URL: &str = "https://auth.openai.com/api/accounts/oauth/revoke";
pub const JWKS_URL: &str = "https://auth.openai.com/.well-known/jwks.json";
pub const ISSUER: &str = "https://auth.openai.com";
pub const RESOURCE: &str = "https://api.openai.com/v1";
pub const DYNAMIC_CLIENT_ID: &str = "dynamic_agent_client";
pub const SCOPE: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
pub const REQUIRED_SCOPE: &str = "chatgpt.tokens.use.direct";
pub const CALLBACK_PATH: &str = "/auth/callback";

fn random_token(bytes: usize) -> AppResult<String> {
    let mut buffer = vec![0u8; bytes];
    getrandom::fill(&mut buffer)
        .map_err(|_| AppError::Authentication("secure random source unavailable".to_owned()))?;
    Ok(URL_SAFE_NO_PAD.encode(buffer))
}

pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Fresh per-attempt secrets. Never log or serialize this value.
pub struct AuthAttempt {
    pub state: String,
    pub nonce: String,
    pub verifier: String,
}

impl AuthAttempt {
    pub fn new() -> AppResult<Self> {
        Ok(Self {
            state: random_token(32)?,
            nonce: random_token(32)?,
            verifier: random_token(48)?,
        })
    }
}

/// Stable per-installation identifier (`urn:uuid:` form allowed by the docs).
pub fn new_host_id() -> AppResult<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|_| AppError::Authentication("secure random source unavailable".to_owned()))?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(format!(
        "urn:uuid:{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    ))
}

pub fn redirect_uri(port: u16) -> String {
    format!("http://127.0.0.1:{port}{CALLBACK_PATH}")
}

/// `client_id` is `None` for first-time dynamic registration, in which case
/// `agent_name_hint` is sent; returning users reuse the issued id and omit it.
pub fn authorize_url(
    client_id: Option<&str>,
    agent_name_hint: &str,
    host_id: &str,
    redirect: &str,
    attempt: &AuthAttempt,
) -> AppResult<String> {
    let mut params = vec![
        ("client_id", client_id.unwrap_or(DYNAMIC_CLIENT_ID)),
        ("ext_agent_host_id", host_id),
        ("redirect_uri", redirect),
        ("response_type", "code"),
        ("scope", SCOPE),
        ("resource", RESOURCE),
        ("state", attempt.state.as_str()),
        ("nonce", attempt.nonce.as_str()),
        ("code_challenge_method", "S256"),
    ];
    let challenge = pkce_challenge(&attempt.verifier);
    params.push(("code_challenge", challenge.as_str()));
    if client_id.is_none() {
        params.push(("agent_name_hint", agent_name_hint));
    }
    reqwest::Url::parse_with_params(AUTHORIZE_URL, params)
        .map(|url| url.to_string())
        .map_err(|_| AppError::Authentication("authorization URL could not be built".to_owned()))
}

#[derive(Debug, PartialEq, Eq)]
pub struct CallbackResult {
    pub code: String,
    /// Issued client id returned by dynamic registration, when present.
    pub issued_client_id: Option<String>,
}

/// Parses the loopback request target (`/auth/callback?code=...&state=...`).
pub fn parse_callback(target: &str, expected_state: &str) -> AppResult<CallbackResult> {
    let url = reqwest::Url::parse(&format!("http://127.0.0.1{target}"))
        .map_err(|_| AppError::Authentication("malformed callback".to_owned()))?;
    if url.path() != CALLBACK_PATH {
        return Err(AppError::Authentication(
            "unexpected callback path".to_owned(),
        ));
    }
    let mut code = None;
    let mut state = None;
    let mut error = None;
    let mut client_id = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            "error" => error = Some(value.into_owned()),
            "client_id" => client_id = Some(value.into_owned()),
            _ => {}
        }
    }
    if state.as_deref() != Some(expected_state) {
        return Err(AppError::Authentication(
            "callback state mismatch".to_owned(),
        ));
    }
    if let Some(error) = error {
        return Err(AppError::Authentication(format!(
            "authorization was not granted: {error}"
        )));
    }
    let code = code
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AppError::Authentication("callback missing code".to_owned()))?;
    Ok(CallbackResult {
        code,
        issued_client_id: client_id.filter(|value| !value.is_empty()),
    })
}

pub fn exchange_form(
    client_id: &str,
    code: &str,
    verifier: &str,
    redirect: &str,
) -> Vec<(&'static str, String)> {
    vec![
        ("grant_type", "authorization_code".to_owned()),
        ("client_id", client_id.to_owned()),
        ("code", code.to_owned()),
        ("code_verifier", verifier.to_owned()),
        ("redirect_uri", redirect.to_owned()),
        ("resource", RESOURCE.to_owned()),
    ]
}

pub fn refresh_form(client_id: &str, refresh_token: &str) -> Vec<(&'static str, String)> {
    vec![
        ("grant_type", "refresh_token".to_owned()),
        ("client_id", client_id.to_owned()),
        ("refresh_token", refresh_token.to_owned()),
        ("resource", RESOURCE.to_owned()),
    ]
}

pub fn revoke_form(client_id: &str, refresh_token: &str) -> Vec<(&'static str, String)> {
    vec![
        ("client_id", client_id.to_owned()),
        ("token", refresh_token.to_owned()),
        ("token_type_hint", "refresh_token".to_owned()),
    ]
}

#[derive(Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub id_token: Option<String>,
    pub expires_in: u64,
    #[serde(default)]
    pub scope: String,
}

impl TokenResponse {
    pub fn parse(body: &str) -> AppResult<Self> {
        serde_json::from_str(body)
            .map_err(|_| AppError::MalformedResponse("token response was not valid".to_owned()))
    }

    pub fn require_plan_scope(&self) -> AppResult<()> {
        if self
            .scope
            .split_whitespace()
            .any(|scope| scope == REQUIRED_SCOPE)
        {
            Ok(())
        } else {
            Err(AppError::Authentication(
                "ChatGPT plan usage was not granted".to_owned(),
            ))
        }
    }
}

/// Persisted per issued client id as JSON in Credential Manager. Access
/// tokens are deliberately not stored; they live in memory only.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StoredSession {
    pub client_id: String,
    pub host_id: String,
    pub email: String,
    pub refresh_token: String,
}

impl std::fmt::Debug for StoredSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredSession")
            .field("client_id", &self.client_id)
            .finish_non_exhaustive()
    }
}

/// Accepts the issued client id only when it is well formed, never the
/// placeholder, and never different from an already-registered one.
pub fn resolve_client_id(
    previous: Option<&StoredSession>,
    returned: Option<&str>,
) -> AppResult<String> {
    let valid = |id: &str| {
        !id.is_empty()
            && id.len() <= 200
            && id != DYNAMIC_CLIENT_ID
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    };
    let rejected = |text: &str| Err(AppError::Authentication(text.to_owned()));
    match (previous, returned) {
        (Some(session), Some(id)) if session.client_id != id => {
            rejected("ChatGPT issued a different client id")
        }
        (Some(session), _) => Ok(session.client_id.clone()),
        (None, Some(id)) if valid(id) => Ok(id.to_owned()),
        (None, _) => rejected("ChatGPT did not issue a client id"),
    }
}

/// Extracts the request target from a raw loopback HTTP request, accepting
/// only `GET` on the callback path with the exact loopback `Host` header.
pub fn callback_target(request: &str, port: u16) -> Option<String> {
    let mut lines = request.lines();
    let mut parts = lines.next()?.split_whitespace();
    if parts.next()? != "GET" {
        return None;
    }
    let target = parts.next()?;
    let expected = format!("127.0.0.1:{port}");
    let host_ok = lines.any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case("host") && value.trim() == expected
        })
    });
    let path = target.split('?').next().unwrap_or_default();
    (host_ok && path == CALLBACK_PATH).then(|| target.to_owned())
}

#[derive(Deserialize)]
struct IdClaims {
    sub: String,
    nonce: Option<String>,
    email: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Identity {
    pub subject: String,
    pub email: String,
}

/// Validates signature (RS256 via JWKS), issuer, audience, expiry and nonce.
/// Returns the verified identity.
pub fn validate_id_token(
    id_token: &str,
    jwks: &JwkSet,
    client_id: &str,
    nonce: Option<&str>,
) -> AppResult<Identity> {
    let invalid = || AppError::Authentication("ID token validation failed".to_owned());
    let header = jsonwebtoken::decode_header(id_token).map_err(|_| invalid())?;
    if header.alg != Algorithm::RS256 {
        return Err(invalid());
    }
    let kid = header.kid.ok_or_else(invalid)?;
    let jwk = jwks.find(&kid).ok_or_else(invalid)?;
    let key = DecodingKey::from_jwk(jwk).map_err(|_| invalid())?;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&[ISSUER]);
    validation.set_audience(&[client_id]);
    validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    let claims = jsonwebtoken::decode::<IdClaims>(id_token, &key, &validation)
        .map_err(|_| invalid())?
        .claims;
    if let Some(expected) = nonce {
        if claims.nonce.as_deref() != Some(expected) {
            return Err(invalid());
        }
    }
    Ok(Identity {
        subject: claims.sub,
        email: claims.email.unwrap_or_default().chars().take(200).collect(),
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum RefreshFailure {
    /// Tokens are unusable: clear them and repeat the full OAuth flow.
    ReauthRequired,
    /// Client configuration is wrong; do not wipe tokens.
    InvalidClient,
    /// Network or server trouble: keep credentials and retry later.
    Transient,
}

/// Short, token-free summary of an OAuth error body for diagnostics.
pub fn error_summary(body: &str) -> String {
    let value = serde_json::from_str::<serde_json::Value>(body).ok();
    let field = |value: &serde_json::Value, name: &str| {
        value
            .get(name)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    let text = value
        .as_ref()
        .and_then(|value| {
            let error = value.get("error")?;
            let code = error
                .as_str()
                .map(str::to_owned)
                .or_else(|| field(error, "code"))?;
            let detail = field(value, "error_description").or_else(|| field(error, "message"));
            Some(match detail {
                Some(detail) => format!("{code}: {detail}"),
                None => code,
            })
        })
        .unwrap_or_else(|| "no error code".to_owned());
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(160)
        .collect()
}

pub fn classify_refresh_failure(status: u16, body: &str) -> RefreshFailure {
    const REAUTH: [&str; 6] = [
        "invalid_grant",
        "invalid_refresh_token",
        "token_expired",
        "refresh_token_expired",
        "refresh_token_invalidated",
        "refresh_token_reused",
    ];
    if status != 401 && status != 400 {
        return RefreshFailure::Transient;
    }
    let code = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            let error = value.get("error")?;
            error
                .as_str()
                .or_else(|| error.get("code").and_then(serde_json::Value::as_str))
                .map(str::to_owned)
        })
        .unwrap_or_default();
    if code == "invalid_client" {
        RefreshFailure::InvalidClient
    } else if REAUTH.contains(&code.as_str()) {
        RefreshFailure::ReauthRequired
    } else {
        RefreshFailure::Transient
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_matches_rfc7636_vector() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn attempts_are_unique_and_url_safe() {
        let a = AuthAttempt::new().expect("attempt");
        let b = AuthAttempt::new().expect("attempt");
        assert_ne!(a.state, b.state);
        assert_ne!(a.verifier, b.verifier);
        assert!(a.verifier.len() >= 43);
        assert!(!a.state.contains(['+', '/', '=']));
    }

    #[test]
    fn host_id_is_uuid_urn() {
        let id = new_host_id().expect("host id");
        assert!(id.starts_with("urn:uuid:"));
        assert_eq!(id.len(), "urn:uuid:".len() + 36);
        assert_eq!(&id[9 + 14..9 + 15], "4");
    }

    #[test]
    fn first_registration_url_uses_dynamic_client_and_hint() {
        let attempt = AuthAttempt::new().expect("attempt");
        let url = authorize_url(None, "MetaPic", "urn:uuid:x", &redirect_uri(1455), &attempt)
            .expect("url");
        assert!(url.starts_with(AUTHORIZE_URL));
        assert!(url.contains("client_id=dynamic_agent_client"));
        assert!(url.contains("agent_name_hint=MetaPic"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("127.0.0.1%3A1455%2Fauth%2Fcallback"));
        assert!(!url.contains(&attempt.verifier));
    }

    #[test]
    fn returning_user_url_reuses_client_and_omits_hint() {
        let attempt = AuthAttempt::new().expect("attempt");
        let url = authorize_url(
            Some("oaiapp_issued"),
            "MetaPic",
            "urn:uuid:x",
            &redirect_uri(1455),
            &attempt,
        )
        .expect("url");
        assert!(url.contains("client_id=oaiapp_issued"));
        assert!(!url.contains("agent_name_hint"));
    }

    #[test]
    fn callback_requires_matching_state_and_code() {
        let ok = parse_callback("/auth/callback?code=abc&state=s1&client_id=oaiapp_1", "s1")
            .expect("callback");
        assert_eq!(ok.code, "abc");
        assert_eq!(ok.issued_client_id.as_deref(), Some("oaiapp_1"));
        assert!(parse_callback("/auth/callback?code=abc&state=bad", "s1").is_err());
        assert!(parse_callback("/auth/callback?state=s1", "s1").is_err());
        assert!(parse_callback("/auth/callback?error=access_denied&state=s1", "s1").is_err());
        assert!(parse_callback("/other?code=abc&state=s1", "s1").is_err());
    }

    #[test]
    fn plan_scope_is_required() {
        let missing = TokenResponse::parse(
            r#"{"access_token":"a","expires_in":3600,"scope":"openid offline_access"}"#,
        )
        .expect("parse");
        assert!(missing.require_plan_scope().is_err());
        let granted = TokenResponse::parse(
            r#"{"access_token":"a","expires_in":3600,"scope":"openid chatgpt.tokens.use.direct"}"#,
        )
        .expect("parse");
        assert!(granted.require_plan_scope().is_ok());
        assert!(TokenResponse::parse("not json").is_err());
    }

    #[test]
    fn forms_carry_resource_and_never_a_client_secret() {
        for form in [
            exchange_form("c", "code", "v", "http://127.0.0.1:1/auth/callback"),
            refresh_form("c", "r"),
        ] {
            assert!(form.iter().any(|(k, v)| *k == "resource" && v == RESOURCE));
            assert!(form.iter().all(|(k, _)| *k != "client_secret"));
        }
    }

    #[test]
    fn refresh_failures_follow_documented_recovery() {
        let body = |code: &str| format!(r#"{{"error":"{code}"}}"#);
        for code in [
            "invalid_grant",
            "refresh_token_reused",
            "refresh_token_expired",
        ] {
            assert_eq!(
                classify_refresh_failure(401, &body(code)),
                RefreshFailure::ReauthRequired
            );
        }
        assert_eq!(
            classify_refresh_failure(401, &body("invalid_client")),
            RefreshFailure::InvalidClient
        );
        assert_eq!(
            classify_refresh_failure(503, "upstream down"),
            RefreshFailure::Transient
        );
        assert_eq!(
            classify_refresh_failure(401, "not json"),
            RefreshFailure::Transient
        );
    }

    #[test]
    fn error_summary_reports_code_without_leaking_other_fields() {
        assert_eq!(
            error_summary(
                r#"{"error":"invalid_request","error_description":"bad resource","refresh_token":"SECRET"}"#
            ),
            "invalid_request: bad resource"
        );
        assert_eq!(error_summary("<html>blocked</html>"), "no error code");
    }

    #[test]
    fn id_token_with_unknown_key_or_wrong_alg_is_rejected() {
        let jwks = JwkSet { keys: Vec::new() };
        assert!(validate_id_token("garbage", &jwks, "c", None).is_err());
        // HS256 header must never be accepted.
        let hs = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCIsImtpZCI6ImsxIn0.e30.sig";
        assert!(validate_id_token(hs, &jwks, "c", None).is_err());
    }

    #[test]
    fn session_debug_hides_tokens() {
        let session = StoredSession {
            client_id: "c".to_owned(),
            host_id: "h".to_owned(),
            email: "me@example.com".to_owned(),
            refresh_token: "SECRET-REFRESH".to_owned(),
        };
        let text = format!("{session:?}");
        assert!(!text.contains("SECRET") && !text.contains("example.com"));
    }

    #[test]
    fn client_id_rules() {
        assert_eq!(
            resolve_client_id(None, Some("oaiapp_abc-1")).expect("valid"),
            "oaiapp_abc-1"
        );
        assert!(resolve_client_id(None, Some(DYNAMIC_CLIENT_ID)).is_err());
        assert!(resolve_client_id(None, Some("bad id")).is_err());
        assert!(resolve_client_id(None, None).is_err());
        let session = StoredSession {
            client_id: "app_1".to_owned(),
            host_id: "h".to_owned(),
            email: String::new(),
            refresh_token: "r".to_owned(),
        };
        assert_eq!(
            resolve_client_id(Some(&session), None).expect("kept"),
            "app_1"
        );
        assert!(resolve_client_id(Some(&session), Some("app_2")).is_err());
    }

    #[test]
    fn callback_target_requires_get_loopback_host_and_path() {
        let request = |line: &str, host: &str| {
            format!(
                "{line}
Host: {host}

"
            )
        };
        let ok = request(
            "GET /auth/callback?code=a&state=b HTTP/1.1",
            "127.0.0.1:47836",
        );
        assert_eq!(
            callback_target(&ok, 47836).as_deref(),
            Some("/auth/callback?code=a&state=b")
        );
        let evil = request("GET /auth/callback?code=a HTTP/1.1", "evil.test");
        assert!(callback_target(&evil, 47836).is_none());
        let path = request("GET /other HTTP/1.1", "127.0.0.1:47836");
        assert!(callback_target(&path, 47836).is_none());
        let post = request("POST /auth/callback HTTP/1.1", "127.0.0.1:47836");
        assert!(callback_target(&post, 47836).is_none());
    }
}
