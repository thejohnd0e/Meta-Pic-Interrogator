//! Google Gemini API (API key from Google AI Studio): image description and
//! model discovery over the public REST API.
use std::io::{BufRead, Read};
use std::time::Duration;

use base64::Engine;
use serde_json::{json, Value};

use crate::domain::{AppError, AppResult, VisionModel};
use crate::providers::{Transport, VisionRequest, VisionResponse};

pub const PROVIDER_ID: &str = "gemini";
pub const DEFAULT_ENDPOINT: &str = "https://generativelanguage.googleapis.com";
const MAX_STREAM_BYTES: usize = 4 * 1024 * 1024;
const MAX_ERROR_BYTES: usize = 64 * 1024;

fn base_url(endpoint: &str) -> AppResult<String> {
    let endpoint = endpoint.trim().trim_end_matches('/');
    let endpoint = if endpoint.is_empty() {
        DEFAULT_ENDPOINT
    } else {
        endpoint
    };
    let url = reqwest::Url::parse(endpoint)
        .map_err(|_| AppError::InvalidPath("provider endpoint is invalid".to_owned()))?;
    let local_http =
        url.scheme() == "http" && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
    if url.scheme() != "https" && !local_http {
        return Err(AppError::InvalidPath(
            "provider endpoint must use HTTPS or local HTTP".to_owned(),
        ));
    }
    Ok(endpoint.to_owned())
}

/// Accepts `gemini-x` or `models/gemini-x`; rejects anything that could alter the URL path.
pub fn model_id(model: &str) -> AppResult<String> {
    let id = model.trim().trim_start_matches("models/");
    let valid = !id.is_empty()
        && id.len() <= 100
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'));
    if valid {
        Ok(id.to_owned())
    } else {
        Err(AppError::UnavailableModel(
            "Gemini model id is invalid".to_owned(),
        ))
    }
}

pub fn build_payload(prompt: &str, mime: &str, encoded: &str) -> Value {
    json!({
        "contents": [{
            "role": "user",
            "parts": [
                {"text": prompt},
                {"inline_data": {"mime_type": mime, "data": encoded}}
            ]
        }]
    })
}

fn clean(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(160)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Maps an HTTP error and its (optional) JSON body to an app error without echoing the key.
pub fn map_error(status: u16, body: &str) -> AppError {
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .map(clean)
        })
        .unwrap_or_default();
    match status {
        401 | 403 => AppError::Authentication("Gemini rejected the API key".to_owned()),
        404 => AppError::UnavailableModel("Gemini model was not found".to_owned()),
        429 => AppError::RateLimit("Gemini rate limit or quota reached".to_owned()),
        500..=599 => AppError::Network("Gemini server error".to_owned()),
        400 if message.to_ascii_lowercase().contains("api key") => {
            AppError::Authentication("Gemini rejected the API key".to_owned())
        }
        _ => AppError::MalformedResponse(format!("Gemini returned status {status}: {message}")),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Chunk {
    pub text: String,
    pub total_tokens: Option<u64>,
    pub finish_reason: Option<String>,
}

/// Parses one SSE line from `streamGenerateContent?alt=sse`.
pub fn parse_event(line: &str) -> AppResult<Option<Chunk>> {
    let Some(data) = line.strip_prefix("data:") else {
        return Ok(None);
    };
    let data = data.trim();
    if data.is_empty() {
        return Ok(None);
    }
    let value: Value = serde_json::from_str(data)
        .map_err(|_| AppError::MalformedResponse("Gemini stream event was not JSON".to_owned()))?;
    if value.pointer("/promptFeedback/blockReason").is_some() {
        return Err(AppError::ContentRejected(
            "Gemini blocked the request".to_owned(),
        ));
    }
    let candidate = value.pointer("/candidates/0");
    let text = candidate
        .and_then(|candidate| candidate.pointer("/content/parts"))
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<String>()
        })
        .unwrap_or_default();
    Ok(Some(Chunk {
        text,
        total_tokens: value
            .pointer("/usageMetadata/totalTokenCount")
            .and_then(Value::as_u64),
        finish_reason: candidate
            .and_then(|candidate| candidate.get("finishReason"))
            .and_then(Value::as_str)
            .map(str::to_owned),
    }))
}

pub fn read_stream<R: BufRead>(
    mut reader: R,
    model: &str,
    on_delta: &mut dyn FnMut(String),
) -> AppResult<VisionResponse> {
    let mut line = String::new();
    let mut total = 0usize;
    let mut text = String::new();
    let mut usage = None;
    let mut finish = None;
    loop {
        line.clear();
        let read = reader
            .read_line(&mut line)
            .map_err(|_| AppError::Network("Gemini stream was interrupted".to_owned()))?;
        if read == 0 {
            break;
        }
        total += read;
        if total > MAX_STREAM_BYTES {
            return Err(AppError::MalformedResponse(
                "Gemini response exceeded size limit".to_owned(),
            ));
        }
        if let Some(chunk) = parse_event(line.trim_end())? {
            if !chunk.text.is_empty() {
                text.push_str(&chunk.text);
                on_delta(chunk.text);
            }
            usage = chunk.total_tokens.or(usage);
            finish = chunk.finish_reason.or(finish);
        }
    }
    if text.trim().is_empty() {
        return Err(match finish.as_deref() {
            Some("STOP") | None => {
                AppError::MalformedResponse("Gemini response had no text".to_owned())
            }
            Some(reason) => AppError::ContentRejected(format!(
                "Gemini returned no text (finish reason {})",
                clean(reason)
            )),
        });
    }
    Ok(VisionResponse {
        text,
        model: model.to_owned(),
        usage,
    })
}

pub struct GeminiTransport {
    client: reqwest::blocking::Client,
    endpoint: String,
    api_key: String,
    model: String,
}

impl GeminiTransport {
    pub fn new(endpoint: &str, api_key: &str, model: &str) -> AppResult<Self> {
        if api_key.trim().is_empty() {
            return Err(AppError::Authentication(
                "Gemini API key is not configured".to_owned(),
            ));
        }
        let client = crate::network::client_builder()?
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|_| AppError::Network("Gemini client unavailable".to_owned()))?;
        Ok(Self {
            client,
            endpoint: base_url(endpoint)?,
            api_key: api_key.trim().to_owned(),
            model: model_id(model)?,
        })
    }
}

impl Transport for GeminiTransport {
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
        let payload = build_payload(&request.prompt, &request.mime, &encoded);
        let url = format!(
            "{}/v1beta/models/{}:streamGenerateContent?alt=sse",
            self.endpoint, self.model
        );
        let response = self
            .client
            .post(url)
            .header("x-goog-api-key", &self.api_key)
            .json(&payload)
            .send()
            .map_err(|_| AppError::Network("Gemini request failed".to_owned()))?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            let mut bytes = Vec::new();
            let _ = response
                .take(MAX_ERROR_BYTES as u64)
                .read_to_end(&mut bytes);
            return Err(map_error(status, &String::from_utf8_lossy(&bytes)));
        }
        read_stream(std::io::BufReader::new(response), &self.model, on_delta)
    }
}

/// Models that can generate text from an image prompt. Embedding, speech,
/// image-generation, and live/audio models are left out.
pub fn parse_models(value: &Value) -> AppResult<Vec<VisionModel>> {
    let models = value
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            AppError::MalformedResponse("Gemini model list has an unexpected shape".to_owned())
        })?;
    const EXCLUDED: [&str; 8] = [
        "embedding",
        "tts",
        "image",
        "live",
        "audio",
        "robotics",
        "computer-use",
        "imagen",
    ];
    Ok(models
        .iter()
        .filter(|model| {
            model
                .get("supportedGenerationMethods")
                .and_then(Value::as_array)
                .is_some_and(|methods| {
                    methods
                        .iter()
                        .any(|m| m.as_str() == Some("generateContent"))
                })
        })
        .filter_map(|model| {
            let id = model
                .get("name")
                .and_then(Value::as_str)?
                .trim_start_matches("models/");
            let lower = id.to_ascii_lowercase();
            let usable = lower.starts_with("gemini")
                && !EXCLUDED.iter().any(|word| lower.contains(word))
                && model_id(id).is_ok();
            usable.then(|| VisionModel {
                id: id.to_owned(),
                display_name: model
                    .get("displayName")
                    .and_then(Value::as_str)
                    .unwrap_or(id)
                    .chars()
                    .take(120)
                    .collect(),
                vision_capable: true,
            })
        })
        .collect())
}

pub fn list_models(endpoint: &str, api_key: &str) -> AppResult<Vec<VisionModel>> {
    if api_key.trim().is_empty() {
        return Err(AppError::Authentication(
            "Gemini API key is not configured".to_owned(),
        ));
    }
    let response = crate::network::client_builder()?
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| AppError::Network("Gemini client unavailable".to_owned()))?
        .get(format!(
            "{}/v1beta/models?pageSize=1000",
            base_url(endpoint)?
        ))
        .header("x-goog-api-key", api_key.trim())
        .send()
        .map_err(|_| AppError::Network("Gemini model list unavailable".to_owned()))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        let mut bytes = Vec::new();
        let _ = response
            .take(MAX_ERROR_BYTES as u64)
            .read_to_end(&mut bytes);
        return Err(map_error(status, &String::from_utf8_lossy(&bytes)));
    }
    let value: Value = response
        .json()
        .map_err(|_| AppError::MalformedResponse("Gemini model list was not JSON".to_owned()))?;
    let models = parse_models(&value)?;
    if models.is_empty() {
        return Err(AppError::UnavailableModel(
            "Gemini returned no usable models for this key".to_owned(),
        ));
    }
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_ids_cannot_change_the_url_path() {
        assert_eq!(
            model_id("models/gemini-2.5-flash").expect("ok"),
            "gemini-2.5-flash"
        );
        assert_eq!(model_id(" gemini-3.1-pro ").expect("ok"), "gemini-3.1-pro");
        for bad in ["", "a/b", "a?x=1", "a b", "../x", "a:generate"] {
            assert!(model_id(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn payload_carries_prompt_and_inline_image_without_a_model_field() {
        let payload = build_payload("Describe", "image/png", "QUJD");
        assert!(payload.get("model").is_none());
        assert_eq!(payload["contents"][0]["parts"][0]["text"], "Describe");
        assert_eq!(
            payload["contents"][0]["parts"][1]["inline_data"]["mime_type"],
            "image/png"
        );
        assert_eq!(
            payload["contents"][0]["parts"][1]["inline_data"]["data"],
            "QUJD"
        );
    }

    #[test]
    fn stream_collects_text_and_usage() {
        let stream = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"A \"}]}}]}\n\n\
data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"cat.\"}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"totalTokenCount\":12}}\n\n";
        let mut seen = Vec::new();
        let response = read_stream(stream.as_bytes(), "m", &mut |d| seen.push(d)).expect("ok");
        assert_eq!(response.text, "A cat.");
        assert_eq!(response.usage, Some(12));
        assert_eq!(seen, ["A ", "cat."]);
    }

    #[test]
    fn blocked_or_empty_responses_are_distinguished() {
        let blocked = "data: {\"promptFeedback\":{\"blockReason\":\"SAFETY\"}}\n";
        assert!(matches!(
            read_stream(blocked.as_bytes(), "m", &mut |_| {}),
            Err(AppError::ContentRejected(_))
        ));
        let safety = "data: {\"candidates\":[{\"finishReason\":\"SAFETY\"}]}\n";
        assert!(matches!(
            read_stream(safety.as_bytes(), "m", &mut |_| {}),
            Err(AppError::ContentRejected(_))
        ));
        assert!(matches!(
            read_stream("".as_bytes(), "m", &mut |_| {}),
            Err(AppError::MalformedResponse(_))
        ));
        assert!(parse_event("event: ping").expect("ignored").is_none());
    }

    #[test]
    fn http_errors_map_without_leaking_bodies() {
        let key_error = r#"{"error":{"code":400,"message":"API key not valid. Please pass a valid API key.","status":"INVALID_ARGUMENT"}}"#;
        assert!(matches!(
            map_error(400, key_error),
            AppError::Authentication(_)
        ));
        assert!(matches!(map_error(403, "{}"), AppError::Authentication(_)));
        assert!(matches!(
            map_error(404, "{}"),
            AppError::UnavailableModel(_)
        ));
        assert!(matches!(map_error(429, "{}"), AppError::RateLimit(_)));
        assert!(matches!(map_error(503, "{}"), AppError::Network(_)));
        match map_error(400, r#"{"error":{"message":"bad\nfield"}}"#) {
            AppError::MalformedResponse(text) => {
                assert_eq!(text, "Gemini returned status 400: bad field")
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn model_list_keeps_only_generating_gemini_models() {
        let models = parse_models(&json!({"models":[
            {"name":"models/gemini-2.5-flash","displayName":"Gemini 2.5 Flash","supportedGenerationMethods":["generateContent","countTokens"]},
            {"name":"models/gemini-embedding-001","supportedGenerationMethods":["embedContent"]},
            {"name":"models/gemini-2.5-flash-preview-tts","supportedGenerationMethods":["generateContent"]},
            {"name":"models/gemini-2.5-flash-image","supportedGenerationMethods":["generateContent"]},
            {"name":"models/imagen-4.0","supportedGenerationMethods":["predict"]},
            {"name":"models/gemma-3-27b-it","supportedGenerationMethods":["generateContent"]}
        ]}))
        .expect("models");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "gemini-2.5-flash");
        assert_eq!(models[0].display_name, "Gemini 2.5 Flash");
        assert!(parse_models(&json!({})).is_err());
    }

    #[test]
    fn endpoints_must_be_https_or_local() {
        assert_eq!(base_url("").expect("default"), DEFAULT_ENDPOINT);
        assert!(base_url("http://example.com").is_err());
        assert!(base_url("http://127.0.0.1:8080").is_ok());
    }
}
