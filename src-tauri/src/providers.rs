use std::io::{BufRead, Read};
use std::sync::{Arc, Mutex};

use base64::Engine;

use crate::domain::{AppError, AppResult, ProviderConfig, VisionCapabilities, VisionModel};

const MAX_PROVIDER_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiProviderKind {
    OpenAi,
    Anthropic,
    Gemini,
    OpenRouter,
    OpenAiCompatible,
}

pub fn build_vision_payload(
    kind: ApiProviderKind,
    model: &str,
    prompt: &str,
    mime: &str,
    encoded_image: &str,
) -> serde_json::Value {
    match kind {
        ApiProviderKind::OpenAi
        | ApiProviderKind::OpenRouter
        | ApiProviderKind::OpenAiCompatible => {
            serde_json::json!({"model": model, "messages": [{"role": "user", "content": [{"type": "text", "text": prompt}, {"type": "image_url", "image_url": {"url": format!("data:{mime};base64,{encoded_image}")}}]}]})
        }
        ApiProviderKind::Anthropic => {
            serde_json::json!({"model": model, "messages": [{"role": "user", "content": [{"type": "text", "text": prompt}, {"type": "image", "source": {"type": "base64", "media_type": mime, "data": encoded_image}}]}]})
        }
        ApiProviderKind::Gemini => {
            serde_json::json!({"model": model, "contents": [{"parts": [{"text": prompt}, {"inline_data": {"mime_type": mime, "data": encoded_image}}]}]})
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisionRequest {
    pub prompt: String,
    pub mime: String,
    pub image: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisionResponse {
    pub text: String,
    pub model: String,
    pub usage: Option<u64>,
}

pub trait VisionBackend: Send + Sync {
    fn provider_id(&self) -> String;
    fn capabilities(&self) -> VisionCapabilities;
    fn models(&self) -> AppResult<Vec<VisionModel>>;
    fn describe(&self, request: VisionRequest) -> AppResult<VisionResponse>;
    fn describe_stream(
        &self,
        request: VisionRequest,
        on_delta: &mut dyn FnMut(String),
    ) -> AppResult<VisionResponse> {
        let response = self.describe(request)?;
        on_delta(response.text.clone());
        Ok(response)
    }
}

pub trait Transport: Send + Sync {
    fn request(&self, provider: &str, request: &VisionRequest) -> AppResult<VisionResponse>;
    fn stream_request(
        &self,
        provider: &str,
        request: &VisionRequest,
        on_delta: &mut dyn FnMut(String),
    ) -> AppResult<VisionResponse> {
        let response = self.request(provider, request)?;
        on_delta(response.text.clone());
        Ok(response)
    }
}

pub fn completion_endpoint(endpoint: &str) -> String {
    let endpoint = endpoint.trim_end_matches('/');
    if endpoint.ends_with("/chat/completions") {
        endpoint.to_owned()
    } else {
        format!("{endpoint}/v1/chat/completions")
    }
}

fn validate_endpoint(endpoint: &str) -> AppResult<()> {
    let url = reqwest::Url::parse(endpoint)
        .map_err(|_| AppError::InvalidPath("provider endpoint is invalid".to_owned()))?;
    let local_http =
        url.scheme() == "http" && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
    if url.scheme() != "https" && !local_http {
        return Err(AppError::InvalidPath(
            "provider endpoint must use HTTPS or local HTTP".to_owned(),
        ));
    }
    Ok(())
}

pub fn parse_openai_response(
    value: &serde_json::Value,
    fallback_model: &str,
) -> AppResult<VisionResponse> {
    let text = value
        .get("choices")
        .and_then(serde_json::Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(serde_json::Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| AppError::MalformedResponse("provider response had no text".to_owned()))?;
    let model = value
        .get("model")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(fallback_model);
    let usage = value
        .get("usage")
        .and_then(|usage| usage.get("total_tokens"))
        .and_then(serde_json::Value::as_u64);
    Ok(VisionResponse {
        text: text.to_owned(),
        model: model.to_owned(),
        usage,
    })
}

pub fn bounded_body(body: &[u8], limit: usize) -> AppResult<String> {
    if body.len() > limit {
        return Err(AppError::MalformedResponse(
            "provider response exceeded size limit".to_owned(),
        ));
    }
    String::from_utf8(body.to_vec())
        .map_err(|_| AppError::MalformedResponse("provider response was not UTF-8".to_owned()))
}

pub fn parse_openai_sse(stream: &str) -> AppResult<String> {
    let mut text = String::new();
    for line in stream.lines() {
        let Some(content) = parse_openai_sse_line(line)? else {
            if line.trim() == "data: [DONE]" {
                break;
            }
            continue;
        };
        text.push_str(&content);
    }
    if text.is_empty() {
        return Err(AppError::MalformedResponse(
            "provider stream had no text".to_owned(),
        ));
    }
    Ok(text)
}

fn parse_openai_sse_line(line: &str) -> AppResult<Option<String>> {
    let Some(data) = line.strip_prefix("data:") else {
        return Ok(None);
    };
    let data = data.trim();
    if data == "[DONE]" {
        return Ok(None);
    }
    let value: serde_json::Value = serde_json::from_str(data).map_err(|_| {
        AppError::MalformedResponse("provider stream returned invalid JSON".to_owned())
    })?;
    Ok(value
        .get("choices")
        .and_then(serde_json::Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("delta"))
        .and_then(|delta| delta.get("content"))
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned))
}

pub struct OpenAiTransport {
    client: reqwest::blocking::Client,
    endpoint: String,
    api_key: String,
    model: String,
}

impl OpenAiTransport {
    pub fn new(endpoint: &str, api_key: &str, model: &str) -> AppResult<Self> {
        if endpoint.trim().is_empty() || api_key.trim().is_empty() || model.trim().is_empty() {
            return Err(AppError::InvalidPath(
                "provider endpoint, credential, and model are required".to_owned(),
            ));
        }
        validate_endpoint(endpoint)?;
        let client = crate::network::client_builder()?
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|_| AppError::Network("provider client unavailable".to_owned()))?;
        Ok(Self {
            client,
            endpoint: completion_endpoint(endpoint),
            api_key: api_key.to_owned(),
            model: model.to_owned(),
        })
    }
}

impl Transport for OpenAiTransport {
    fn request(&self, _provider: &str, request: &VisionRequest) -> AppResult<VisionResponse> {
        let encoded = base64::engine::general_purpose::STANDARD.encode(&request.image);
        let payload = build_vision_payload(
            ApiProviderKind::OpenAi,
            &self.model,
            &request.prompt,
            &request.mime,
            &encoded,
        );
        let response = self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .json(&payload)
            .send()
            .map_err(|_| AppError::Network("provider request failed".to_owned()))?;
        let status = response.status().as_u16();
        let mut response = response;
        let mut bytes = Vec::new();
        response
            .by_ref()
            .take((MAX_PROVIDER_RESPONSE_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| AppError::Network("provider response unreadable".to_owned()))?;
        let body = bounded_body(&bytes, MAX_PROVIDER_RESPONSE_BYTES)?;
        if !(200..300).contains(&status) {
            return Err(match status {
                401 | 403 => AppError::Authentication("provider rejected credentials".to_owned()),
                429 => AppError::RateLimit("provider rate limit".to_owned()),
                500..=599 => AppError::Network("provider server error".to_owned()),
                _ => AppError::MalformedResponse(format!("provider returned status {status}")),
            });
        }
        let value: serde_json::Value = serde_json::from_str(&body).map_err(|_| {
            AppError::MalformedResponse("provider returned invalid JSON".to_owned())
        })?;
        parse_openai_response(&value, &self.model)
    }

    fn stream_request(
        &self,
        _provider: &str,
        request: &VisionRequest,
        on_delta: &mut dyn FnMut(String),
    ) -> AppResult<VisionResponse> {
        let encoded = base64::engine::general_purpose::STANDARD.encode(&request.image);
        let mut payload = build_vision_payload(
            ApiProviderKind::OpenAi,
            &self.model,
            &request.prompt,
            &request.mime,
            &encoded,
        );
        payload["stream"] = serde_json::Value::Bool(true);
        let response = self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .json(&payload)
            .send()
            .map_err(|_| AppError::Network("provider request failed".to_owned()))?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(match status {
                401 | 403 => AppError::Authentication("provider rejected credentials".to_owned()),
                429 => AppError::RateLimit("provider rate limit".to_owned()),
                500..=599 => AppError::Network("provider server error".to_owned()),
                _ => AppError::MalformedResponse(format!("provider returned status {status}")),
            });
        }
        let mut reader = std::io::BufReader::new(response);
        let mut line = String::new();
        let mut total_bytes = 0usize;
        let mut text = String::new();
        loop {
            line.clear();
            let read = reader
                .read_line(&mut line)
                .map_err(|_| AppError::Network("provider stream unreadable".to_owned()))?;
            if read == 0 {
                break;
            }
            total_bytes += read;
            if total_bytes > MAX_PROVIDER_RESPONSE_BYTES {
                return Err(AppError::MalformedResponse(
                    "provider response exceeded size limit".to_owned(),
                ));
            }
            if line.trim() == "data: [DONE]" {
                break;
            }
            if let Some(delta) = parse_openai_sse_line(line.trim_end())? {
                text.push_str(&delta);
                on_delta(delta);
            }
        }
        if text.is_empty() {
            return Err(AppError::MalformedResponse(
                "provider stream had no text".to_owned(),
            ));
        }
        Ok(VisionResponse {
            text,
            model: self.model.clone(),
            usage: None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct MockTransport {
    pub status: u16,
    pub body: String,
}

impl Transport for MockTransport {
    fn request(&self, provider: &str, _request: &VisionRequest) -> AppResult<VisionResponse> {
        match self.status {
            200..=299 => Ok(VisionResponse {
                text: self.body.clone(),
                model: provider.to_owned(),
                usage: None,
            }),
            401 => Err(AppError::Authentication(
                "provider rejected credentials".to_owned(),
            )),
            403 => Err(AppError::Authentication(
                "provider denied access".to_owned(),
            )),
            429 => Err(AppError::RateLimit("provider rate limit".to_owned())),
            500..=599 => Err(AppError::Network("provider server error".to_owned())),
            _ => Err(AppError::MalformedResponse(format!(
                "provider returned status {}",
                self.status
            ))),
        }
    }
}

pub struct ApiProvider<T> {
    id: String,
    transport: T,
    models: Vec<VisionModel>,
}

impl<T: Transport> ApiProvider<T> {
    pub fn new(id: impl Into<String>, transport: T, models: Vec<VisionModel>) -> Self {
        Self {
            id: id.into(),
            transport,
            models,
        }
    }

    pub fn new_with_model(id: impl Into<String>, transport: T, model: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            transport,
            models: vec![VisionModel {
                id: model.into(),
                display_name: "Configured vision model".to_owned(),
                vision_capable: true,
            }],
        }
    }
}

pub fn build_openai_backend(
    config: &ProviderConfig,
    api_key: &str,
) -> AppResult<Box<dyn VisionBackend>> {
    if !matches!(config.provider_id.as_str(), "openai" | "openai-compatible") {
        return Err(AppError::UnavailableModel(config.provider_id.clone()));
    }
    let endpoint = config
        .endpoint
        .as_deref()
        .unwrap_or("https://api.openai.com");
    Ok(Box::new(ApiProvider::new_with_model(
        config.provider_id.clone(),
        OpenAiTransport::new(endpoint, api_key, &config.model_id)?,
        config.model_id.clone(),
    )))
}

pub fn provider_capabilities(provider_id: &str) -> VisionCapabilities {
    let image_input = matches!(
        provider_id,
        "openai" | "openai-compatible" | "chatgpt" | "xai" | "gemini"
    );
    VisionCapabilities {
        image_input,
        streaming: image_input,
        usage_reporting: image_input,
    }
}

/// `{endpoint}/v1/models`, tolerating endpoints that already end in `/v1` or `/chat/completions`.
pub fn models_endpoint(endpoint: &str) -> String {
    let endpoint = endpoint.trim().trim_end_matches('/');
    let endpoint = endpoint
        .strip_suffix("/chat/completions")
        .unwrap_or(endpoint);
    if endpoint.ends_with("/v1") {
        format!("{endpoint}/models")
    } else {
        format!("{endpoint}/v1/models")
    }
}

/// With `chat_only`, keeps GPT and o-series chat models and drops audio, image,
/// embedding, moderation, and similar families.
pub fn parse_openai_models(
    value: &serde_json::Value,
    chat_only: bool,
) -> AppResult<Vec<VisionModel>> {
    const EXCLUDED: [&str; 10] = [
        "audio",
        "realtime",
        "transcribe",
        "tts",
        "image",
        "embedding",
        "moderation",
        "search",
        "instruct",
        "whisper",
    ];
    let data = value
        .get("data")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            AppError::MalformedResponse("provider model list has an unexpected shape".to_owned())
        })?;
    let mut models: Vec<VisionModel> = data
        .iter()
        .filter_map(|model| model.get("id").and_then(serde_json::Value::as_str))
        .filter(|id| !id.is_empty() && id.len() <= 200)
        .filter(|id| {
            if !chat_only {
                return true;
            }
            let lower = id.to_ascii_lowercase();
            let family = lower.starts_with("gpt-")
                || lower.starts_with("chatgpt-")
                || (lower.starts_with('o')
                    && lower.chars().nth(1).is_some_and(|c| c.is_ascii_digit()));
            family && !EXCLUDED.iter().any(|word| lower.contains(word))
        })
        .map(|id| VisionModel {
            id: id.to_owned(),
            display_name: id.to_owned(),
            vision_capable: true,
        })
        .collect();
    models.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(models)
}

pub fn fetch_openai_models(
    endpoint: &str,
    api_key: &str,
    chat_only: bool,
) -> AppResult<Vec<VisionModel>> {
    if api_key.trim().is_empty() {
        return Err(AppError::Authentication(
            "provider credential is not configured".to_owned(),
        ));
    }
    validate_endpoint(endpoint)?;
    let response = crate::network::client_builder()?
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|_| AppError::Network("provider client unavailable".to_owned()))?
        .get(models_endpoint(endpoint))
        .bearer_auth(api_key.trim())
        .send()
        .map_err(|_| AppError::Network("provider model list unavailable".to_owned()))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(match status {
            401 | 403 => AppError::Authentication("provider rejected credentials".to_owned()),
            429 => AppError::RateLimit("provider rate limit".to_owned()),
            500..=599 => AppError::Network("provider server error".to_owned()),
            _ => AppError::MalformedResponse(format!("provider returned status {status}")),
        });
    }
    let value: serde_json::Value = response
        .json()
        .map_err(|_| AppError::MalformedResponse("provider model list was not JSON".to_owned()))?;
    let models = parse_openai_models(&value, chat_only)?;
    if models.is_empty() {
        return Err(AppError::UnavailableModel(
            "provider returned no usable models".to_owned(),
        ));
    }
    Ok(models)
}

pub fn configured_models(config: &ProviderConfig) -> AppResult<Vec<VisionModel>> {
    if !matches!(config.provider_id.as_str(), "openai" | "openai-compatible") {
        return Err(AppError::UnavailableModel(config.provider_id.clone()));
    }
    if config.model_id.trim().is_empty() {
        return Err(AppError::UnavailableModel(
            "provider model is required".to_owned(),
        ));
    }
    Ok(vec![VisionModel {
        id: config.model_id.clone(),
        display_name: "Configured vision model".to_owned(),
        vision_capable: true,
    }])
}

impl<T: Transport> VisionBackend for ApiProvider<T> {
    fn provider_id(&self) -> String {
        self.id.clone()
    }
    fn capabilities(&self) -> VisionCapabilities {
        VisionCapabilities {
            image_input: true,
            streaming: true,
            usage_reporting: true,
        }
    }
    fn models(&self) -> AppResult<Vec<VisionModel>> {
        Ok(self.models.clone())
    }
    fn describe(&self, request: VisionRequest) -> AppResult<VisionResponse> {
        self.transport.request(&self.id, &request)
    }
    fn describe_stream(
        &self,
        request: VisionRequest,
        on_delta: &mut dyn FnMut(String),
    ) -> AppResult<VisionResponse> {
        self.transport.stream_request(&self.id, &request, on_delta)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityState {
    Unknown,
    Available,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeState {
    pub state: CapabilityState,
    pub checked_at_utc: Option<String>,
}

pub fn vision_probe(backend: &dyn VisionBackend, image: Vec<u8>) -> ProbeState {
    let request = VisionRequest {
        prompt: "probe".to_owned(),
        mime: "image/png".to_owned(),
        image,
    };
    let state = match backend.describe(request) {
        Ok(_) => CapabilityState::Available,
        Err(_) => CapabilityState::Unavailable,
    };
    ProbeState {
        state,
        checked_at_utc: Some("now".to_owned()),
    }
}

pub struct RequestRegistry {
    state: Arc<Mutex<RequestState>>,
}

struct RequestState {
    active: bool,
    cancelled: bool,
}

impl Default for RequestRegistry {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(RequestState {
                active: false,
                cancelled: false,
            })),
        }
    }
}

impl RequestRegistry {
    pub fn begin(&self) -> AppResult<RequestGuard> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| AppError::Cancellation("request registry unavailable".to_owned()))?;
        if state.active {
            return Err(AppError::Cancellation(
                "another request is active".to_owned(),
            ));
        }
        state.active = true;
        state.cancelled = false;
        Ok(RequestGuard {
            state: Arc::clone(&self.state),
        })
    }

    pub fn cancel(&self) -> AppResult<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| AppError::Cancellation("request registry unavailable".to_owned()))?;
        if !state.active {
            return Err(AppError::Cancellation("no request is active".to_owned()));
        }
        state.cancelled = true;
        Ok(())
    }

    pub fn is_cancelled(&self) -> AppResult<bool> {
        let state = self
            .state
            .lock()
            .map_err(|_| AppError::Cancellation("request registry unavailable".to_owned()))?;
        Ok(state.cancelled)
    }
}

pub struct RequestGuard {
    state: Arc<Mutex<RequestState>>,
}

impl Drop for RequestGuard {
    fn drop(&mut self) {
        if let Ok(mut state) = self.state.lock() {
            state.active = false;
        }
    }
}

pub fn provider_factory(config: &ProviderConfig) -> Option<Box<dyn VisionBackend>> {
    let known = [
        "openai",
        "anthropic",
        "gemini",
        "openrouter",
        "openai-compatible",
        "xai",
        "deepseek",
        "chatgpt",
        "supergrok",
    ];
    known.contains(&config.provider_id.as_str()).then(|| {
        Box::new(ApiProvider::new(
            config.provider_id.clone(),
            MockTransport {
                status: 200,
                body: "mock".to_owned(),
            },
            Vec::new(),
        )) as Box<dyn VisionBackend>
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> VisionRequest {
        VisionRequest {
            prompt: "p".to_owned(),
            mime: "image/png".to_owned(),
            image: vec![1, 2, 3],
        }
    }

    #[test]
    fn maps_auth_rate_limit_and_server_errors() {
        for (status, category) in [
            (401, "authentication"),
            (429, "rate_limit"),
            (500, "network"),
        ] {
            let result = MockTransport {
                status,
                body: "secret response".to_owned(),
            }
            .request("openai", &request());
            let error = result.expect_err("status should fail");
            let encoded = serde_json::to_string(&error).expect("error serializes");
            assert!(encoded.contains(category));
            assert!(!encoded.contains("secret response"));
        }
    }

    #[test]
    fn registry_allows_only_one_active_request() {
        let registry = RequestRegistry::default();
        let guard = registry.begin().expect("first request begins");
        assert!(registry.begin().is_err());
        drop(guard);
        assert!(registry.begin().is_ok());
    }

    #[test]
    fn probe_disables_failed_vision() {
        let backend = ApiProvider::new(
            "xai",
            MockTransport {
                status: 403,
                body: String::new(),
            },
            Vec::new(),
        );
        assert_eq!(
            vision_probe(&backend, vec![1]).state,
            CapabilityState::Unavailable
        );
    }

    #[test]
    fn provider_payloads_keep_image_mime_and_encoding() {
        let payload = build_vision_payload(
            ApiProviderKind::OpenAi,
            "vision",
            "describe",
            "image/png",
            "abc",
        );
        assert_eq!(
            payload["messages"][0]["content"][1]["image_url"]["url"],
            "data:image/png;base64,abc"
        );
        let anthropic = build_vision_payload(
            ApiProviderKind::Anthropic,
            "vision",
            "describe",
            "image/jpeg",
            "xyz",
        );
        assert_eq!(
            anthropic["messages"][0]["content"][1]["source"]["data"],
            "xyz"
        );
    }

    #[test]
    fn parses_openai_completion_without_exposing_credentials() {
        let response = serde_json::json!({
            "id": "chatcmpl-test",
            "model": "gpt-4.1-mini",
            "choices": [{"message": {"content": "A calm lake."}}],
            "usage": {"total_tokens": 17}
        });
        let parsed = parse_openai_response(&response, "gpt-4.1-mini").expect("response parses");
        assert_eq!(parsed.text, "A calm lake.");
        assert_eq!(parsed.model, "gpt-4.1-mini");
        assert_eq!(parsed.usage, Some(17));
        assert!(!parsed.text.contains("secret"));
    }

    #[test]
    fn openai_endpoint_appends_completion_path_once() {
        assert_eq!(
            completion_endpoint("https://api.openai.com"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            completion_endpoint("https://example.test/custom/chat/completions"),
            "https://example.test/custom/chat/completions"
        );
        assert!(validate_endpoint("https://api.openai.com").is_ok());
        assert!(validate_endpoint("http://localhost:8080").is_ok());
        assert!(validate_endpoint("http://example.test").is_err());
    }

    #[test]
    fn capabilities_are_disabled_for_unverified_providers() {
        assert!(provider_capabilities("openai").image_input);
        assert!(provider_capabilities("openai-compatible").image_input);
        assert!(provider_capabilities("chatgpt").image_input);
        assert!(provider_capabilities("xai").image_input);
        assert!(provider_capabilities("gemini").image_input);
        assert!(!provider_capabilities("anthropic").image_input);
        assert!(!provider_capabilities("deepseek").image_input);
    }

    #[test]
    fn configured_model_refresh_returns_only_verified_model() {
        let config = ProviderConfig {
            provider_id: "openai".to_owned(),
            model_id: "gpt-4.1-mini".to_owned(),
            endpoint: None,
        };
        let models = configured_models(&config).expect("configured model is valid");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "gpt-4.1-mini");
        assert!(models[0].vision_capable);
        assert!(configured_models(&ProviderConfig {
            provider_id: "chatgpt".to_owned(),
            ..config
        })
        .is_err());
    }

    #[test]
    fn cancellation_marks_active_request_and_guard_releases_registry() {
        let registry = RequestRegistry::default();
        let guard = registry.begin().expect("request begins");
        assert!(!registry.is_cancelled().expect("state reads"));
        registry.cancel().expect("request cancels");
        assert!(registry.is_cancelled().expect("state reads"));
        drop(guard);
        assert!(registry.begin().is_ok());
    }

    #[test]
    fn bounded_body_rejects_oversized_provider_response() {
        assert_eq!(bounded_body(b"ok", 2).expect("body fits"), "ok");
        assert!(matches!(
            bounded_body(b"secret-too-large", 6),
            Err(AppError::MalformedResponse(message)) if message == "provider response exceeded size limit"
        ));
    }

    #[test]
    fn parses_openai_sse_deltas_until_done() {
        let stream = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"A calm\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\" lake.\"}}]}\n\n",
            "data: [DONE]\n\n"
        );
        assert_eq!(
            parse_openai_sse(stream).expect("stream parses"),
            "A calm lake."
        );
    }

    #[test]
    fn malformed_sse_never_returns_provider_body() {
        let error = parse_openai_sse("data: {\"secret\":").expect_err("stream fails");
        assert_eq!(
            error,
            AppError::MalformedResponse("provider stream returned invalid JSON".to_owned())
        );
        assert!(!serde_json::to_string(&error).unwrap().contains("secret"));
    }

    #[test]
    fn backend_stream_callback_receives_mock_result() {
        let backend = ApiProvider::new(
            "openai",
            MockTransport {
                status: 200,
                body: "streamed text".to_owned(),
            },
            Vec::new(),
        );
        let mut fragments = Vec::new();
        let response = backend
            .describe_stream(request(), &mut |fragment| fragments.push(fragment))
            .expect("stream succeeds");
        assert_eq!(fragments, vec!["streamed text"]);
        assert_eq!(response.text, "streamed text");
    }
    #[test]
    fn openai_model_list_is_filtered_and_endpoints_are_normalized() {
        let value = serde_json::json!({"data":[
            {"id":"gpt-4.1-mini"},{"id":"o3"},{"id":"whisper-1"},
            {"id":"gpt-4o-audio-preview"},{"id":"text-embedding-3-small"},{"id":"gpt-image-1"}
        ]});
        let chat = parse_openai_models(&value, true).expect("chat");
        assert_eq!(
            chat.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["gpt-4.1-mini", "o3"]
        );
        assert_eq!(parse_openai_models(&value, false).expect("all").len(), 6);
        assert!(parse_openai_models(&serde_json::json!({}), true).is_err());
        assert_eq!(
            models_endpoint("https://api.openai.com"),
            "https://api.openai.com/v1/models"
        );
        assert_eq!(
            models_endpoint("https://x.test/v1/"),
            "https://x.test/v1/models"
        );
        assert_eq!(
            models_endpoint("https://x.test/v1/chat/completions"),
            "https://x.test/v1/models"
        );
    }
}
