//! Optional outbound proxy (HTTP or SOCKS5) shared by every HTTP client.
//!
//! The proxy address and user name live in `settings.json`; the password is
//! kept in the credential store under [`PASSWORD_KEY`].
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

use crate::domain::{AppError, AppResult};

pub const PASSWORD_KEY: &str = "proxy-password";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ProxyKind {
    #[default]
    Http,
    Socks5,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProxySettings {
    pub enabled: bool,
    #[serde(default)]
    pub kind: ProxyKind,
    /// `host:port`, without scheme or credentials.
    #[serde(default)]
    pub address: String,
    #[serde(default)]
    pub username: Option<String>,
}

static PROXY: RwLock<Option<reqwest::Url>> = RwLock::new(None);

fn invalid(text: &str) -> AppError {
    AppError::InvalidPath(text.to_owned())
}

/// Validates the settings and builds the proxy URL, or `None` when disabled.
/// SOCKS5 uses `socks5h` so that DNS is resolved by the proxy.
pub fn proxy_url(
    settings: &ProxySettings,
    password: Option<&str>,
) -> AppResult<Option<reqwest::Url>> {
    if !settings.enabled {
        return Ok(None);
    }
    let address = settings.address.trim();
    if address.is_empty() {
        return Err(invalid("proxy address is required"));
    }
    if address.contains("://") || address.contains('@') || address.contains(['/', '?', '#']) {
        return Err(invalid("proxy address must look like host:port"));
    }
    let scheme = match settings.kind {
        ProxyKind::Http => "http",
        ProxyKind::Socks5 => "socks5h",
    };
    let mut url = reqwest::Url::parse(&format!("{scheme}://{address}"))
        .map_err(|_| invalid("proxy address is invalid"))?;
    if url.host_str().is_none() || url.port().is_none() {
        return Err(invalid("proxy address must include a host and a port"));
    }
    if let Some(user) = settings.username.as_deref().filter(|user| !user.is_empty()) {
        url.set_username(user)
            .map_err(|_| invalid("proxy user name is invalid"))?;
        if let Some(password) = password.filter(|password| !password.is_empty()) {
            url.set_password(Some(password))
                .map_err(|_| invalid("proxy password is invalid"))?;
        }
    }
    Ok(Some(url))
}

/// Replaces the process-wide proxy used by [`client_builder`].
pub fn apply(proxy: Option<reqwest::Url>) {
    if let Ok(mut guard) = PROXY.write() {
        *guard = proxy;
    }
}

pub fn builder_with(proxy: Option<&reqwest::Url>) -> AppResult<reqwest::blocking::ClientBuilder> {
    let builder = reqwest::blocking::Client::builder().user_agent("MetaPic-Interrogator/0.1");
    match proxy {
        Some(url) => Ok(builder.proxy(
            reqwest::Proxy::all(url.as_str()).map_err(|_| invalid("proxy settings are invalid"))?,
        )),
        None => Ok(builder),
    }
}

/// Client builder for all outbound requests, honoring the configured proxy.
pub fn client_builder() -> AppResult<reqwest::blocking::ClientBuilder> {
    let proxy = PROXY.read().ok().and_then(|guard| guard.clone());
    builder_with(proxy.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn settings(kind: ProxyKind, address: &str) -> ProxySettings {
        ProxySettings {
            enabled: true,
            kind,
            address: address.to_owned(),
            username: None,
        }
    }

    #[test]
    fn disabled_proxy_yields_no_url_even_with_junk_fields() {
        let mut value = settings(ProxyKind::Http, "not valid");
        value.enabled = false;
        assert!(proxy_url(&value, None).expect("disabled").is_none());
    }

    #[test]
    fn kinds_map_to_schemes_and_socks_resolves_dns_remotely() {
        let http = proxy_url(&settings(ProxyKind::Http, "127.0.0.1:8080"), None)
            .expect("http")
            .expect("url");
        assert_eq!(http.scheme(), "http");
        let socks = proxy_url(&settings(ProxyKind::Socks5, "proxy.local:1080"), None)
            .expect("socks")
            .expect("url");
        assert_eq!(socks.scheme(), "socks5h");
        assert_eq!(socks.port(), Some(1080));
    }

    #[test]
    fn credentials_are_encoded_and_only_used_with_a_user_name() {
        let mut value = settings(ProxyKind::Http, "127.0.0.1:8080");
        value.username = Some("me@x".to_owned());
        let url = proxy_url(&value, Some("p@ss:/w"))
            .expect("auth")
            .expect("url");
        assert_eq!(url.username(), "me%40x");
        assert_eq!(url.password(), Some("p%40ss%3A%2Fw"));
        value.username = None;
        let plain = proxy_url(&value, Some("secret"))
            .expect("plain")
            .expect("url");
        assert!(plain.password().is_none());
    }

    #[test]
    fn malformed_addresses_are_rejected() {
        for address in [
            "",
            "   ",
            "127.0.0.1",
            "http://127.0.0.1:8080",
            "user@127.0.0.1:8080",
            "127.0.0.1:8080/path",
            "127.0.0.1:notaport",
        ] {
            assert!(
                proxy_url(&settings(ProxyKind::Http, address), None).is_err(),
                "{address}"
            );
        }
    }

    #[test]
    fn requests_are_routed_through_the_http_proxy() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buffer = [0_u8; 2048];
            let count = stream.read(&mut buffer).expect("read");
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
            String::from_utf8_lossy(&buffer[..count]).into_owned()
        });
        let proxy = proxy_url(
            &settings(ProxyKind::Http, &format!("127.0.0.1:{port}")),
            None,
        )
        .expect("proxy")
        .expect("url");
        let client = builder_with(Some(&proxy))
            .expect("builder")
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .expect("client");
        let body = client
            .get("http://metapic-proxy-test.invalid/path")
            .send()
            .expect("response")
            .text()
            .expect("text");
        assert_eq!(body, "ok");
        let request = server.join().expect("server");
        assert!(
            request.starts_with("GET http://metapic-proxy-test.invalid/path"),
            "{request}"
        );
    }
}
