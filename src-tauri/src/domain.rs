use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InputImage {
    pub path: String,
}

impl InputImage {
    pub fn new(path: String) -> Result<Self, AppError> {
        if path.trim().is_empty() {
            return Err(AppError::InvalidPath("image path is empty".to_owned()));
        }
        Ok(Self { path })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImageInfo {
    pub path: String,
    pub format: String,
    pub width: u32,
    pub height: u32,
    pub has_alpha: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    pub provider_id: String,
    pub model_id: String,
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VisionModel {
    pub id: String,
    pub display_name: String,
    pub vision_capable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VisionCapabilities {
    pub image_input: bool,
    pub streaming: bool,
    pub usage_reporting: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DescriptionDraft {
    pub text: String,
    pub is_dirty: bool,
}

impl DescriptionDraft {
    pub fn new(text: String) -> Result<Self, AppError> {
        if text.trim().is_empty() {
            return Err(AppError::EmptyDescription);
        }
        Ok(Self {
            text,
            is_dirty: false,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Provenance {
    pub schema_version: u32,
    pub provider: String,
    pub model: String,
    pub preset_id: String,
    pub preset_name: String,
    pub preset_prompt: String,
    pub created_at_utc: String,
    pub app_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SaveRequest {
    pub source_path: String,
    pub destination_path: String,
    pub description: String,
    pub provenance: Provenance,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "category", content = "message")]
#[serde(rename_all = "snake_case")]
pub enum AppError {
    Authentication(String),
    UnavailableModel(String),
    NoVisionSupport(String),
    RateLimit(String),
    Network(String),
    ContentRejected(String),
    MalformedResponse(String),
    Cancellation(String),
    LocalImage(String),
    LocalMetadata(String),
    InvalidPath(String),
    EmptyDescription,
    NotImplemented(String),
}

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SettingsDocument {
    pub schema_version: u32,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
    pub preset_id: Option<String>,
}

impl SettingsDocument {
    pub fn migrate(raw: serde_json::Value) -> AppResult<Self> {
        let version = raw
            .get("schemaVersion")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        if version > 1 {
            return Err(AppError::MalformedResponse(
                "unsupported settings version".to_owned(),
            ));
        }
        serde_json::from_value(raw)
            .map(|mut settings: Self| {
                settings.schema_version = 1;
                settings
            })
            .map_err(|error| AppError::MalformedResponse(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::{AppError, DescriptionDraft, InputImage, Provenance, SettingsDocument};

    #[test]
    fn provenance_round_trips_as_json() {
        let value = Provenance {
            schema_version: 1,
            provider: "openai".to_owned(),
            model: "vision-model".to_owned(),
            preset_id: "concise".to_owned(),
            preset_name: "Concise".to_owned(),
            preset_prompt: "Describe the image".to_owned(),
            created_at_utc: "2026-10-05T00:00:00Z".to_owned(),
            app_version: "0.1.0".to_owned(),
        };

        let encoded = serde_json::to_string(&value).expect("provenance is serializable");
        let decoded: Provenance = serde_json::from_str(&encoded).expect("provenance is readable");
        assert_eq!(decoded, value);
    }

    #[test]
    fn empty_descriptions_are_rejected() {
        assert_eq!(
            DescriptionDraft::new("  ".to_owned()),
            Err(AppError::EmptyDescription)
        );
    }

    #[test]
    fn empty_image_paths_are_rejected() {
        assert_eq!(
            InputImage::new("\t".to_owned()),
            Err(AppError::InvalidPath("image path is empty".to_owned()))
        );
    }

    #[test]
    fn error_categories_use_stable_wire_names() {
        let encoded = serde_json::to_string(&AppError::NoVisionSupport("model".to_owned()))
            .expect("errors are serializable");
        assert!(encoded.contains("no_vision_support"));
    }

    #[test]
    fn settings_migrate_to_current_schema() {
        let old = serde_json::json!({ "schemaVersion": 0, "presetId": "concise" });
        let migrated = SettingsDocument::migrate(old).expect("known settings versions migrate");
        assert_eq!(migrated.schema_version, 1);
        assert_eq!(migrated.preset_id.as_deref(), Some("concise"));
        assert_eq!(migrated.endpoint, None);
    }
}
