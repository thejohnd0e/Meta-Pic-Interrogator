use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::domain::{AppError, AppResult, Preset, SettingsDocument};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SettingsFile {
    pub schema_version: u32,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
    pub preset_id: Option<String>,
    #[serde(default)]
    pub proxy: Option<crate::network::ProxySettings>,
    #[serde(default)]
    pub model_by_provider: std::collections::BTreeMap<String, String>,
    /// Legacy location of presets; they now live in files (see `presets.rs`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub presets: Vec<Preset>,
}

impl Default for SettingsFile {
    fn default() -> Self {
        Self {
            schema_version: 1,
            provider_id: None,
            model_id: None,
            endpoint: None,
            preset_id: None,
            proxy: None,
            model_by_provider: Default::default(),
            presets: Vec::new(),
        }
    }
}

pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn load(&self) -> AppResult<SettingsFile> {
        if !self.path.exists() {
            return Ok(SettingsFile::default());
        }
        let raw = std::fs::read_to_string(&self.path)
            .map_err(|error| AppError::LocalMetadata(error.to_string()))?;
        serde_json::from_str(&raw).map_err(|error| AppError::MalformedResponse(error.to_string()))
    }

    pub fn save(&self, settings: &SettingsFile) -> AppResult<()> {
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)
            .map_err(|error| AppError::LocalMetadata(error.to_string()))?;
        let json = serde_json::to_vec_pretty(settings)
            .map_err(|error| AppError::LocalMetadata(error.to_string()))?;
        std::fs::write(&self.path, json).map_err(|error| AppError::LocalMetadata(error.to_string()))
    }
}

pub fn migrate_document(document: SettingsDocument) -> SettingsDocument {
    SettingsDocument {
        schema_version: 1,
        ..document
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_settings_do_not_embed_presets() {
        let settings = SettingsFile::default();
        assert_eq!(settings.schema_version, 1);
        assert!(settings.presets.is_empty());
        let json = serde_json::to_string(&settings).expect("serializable");
        assert!(!json.contains("presets"));
    }

    #[test]
    fn legacy_settings_with_presets_still_load() {
        let path =
            std::env::temp_dir().join(format!("metapic-settings-{}.json", std::process::id()));
        std::fs::write(
            &path,
            r#"{"schema_version":1,"provider_id":null,"model_id":null,"preset_id":null,
                "presets":[{"id":"a","name":"A","prompt":"p"}]}"#,
        )
        .expect("write");
        let loaded = SettingsStore::new(&path).load().expect("load");
        assert_eq!(loaded.presets.len(), 1);
        let _ = std::fs::remove_file(path);
    }
}
