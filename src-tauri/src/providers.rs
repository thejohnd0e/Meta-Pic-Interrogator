use std::sync::{Arc, Mutex};

use crate::domain::{AppError, AppResult, ProviderConfig, VisionCapabilities, VisionModel};

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
}

pub trait Transport: Send + Sync {
    fn request(&self, provider: &str, request: &VisionRequest) -> AppResult<VisionResponse>;
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
}

impl<T: Transport> VisionBackend for ApiProvider<T> {
    fn provider_id(&self) -> String {
        self.id.clone()
    }
    fn capabilities(&self) -> VisionCapabilities {
        VisionCapabilities {
            image_input: true,
            streaming: false,
            usage_reporting: true,
        }
    }
    fn models(&self) -> AppResult<Vec<VisionModel>> {
        Ok(self.models.clone())
    }
    fn describe(&self, request: VisionRequest) -> AppResult<VisionResponse> {
        self.transport.request(&self.id, &request)
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
    active: Arc<Mutex<bool>>,
}

impl Default for RequestRegistry {
    fn default() -> Self {
        Self {
            active: Arc::new(Mutex::new(false)),
        }
    }
}

impl RequestRegistry {
    pub fn begin(&self) -> AppResult<RequestGuard> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| AppError::Cancellation("request registry unavailable".to_owned()))?;
        if *active {
            return Err(AppError::Cancellation(
                "another request is active".to_owned(),
            ));
        }
        *active = true;
        Ok(RequestGuard {
            active: Arc::clone(&self.active),
        })
    }
}

pub struct RequestGuard {
    active: Arc<Mutex<bool>>,
}

impl Drop for RequestGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active.lock() {
            *active = false;
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
}
