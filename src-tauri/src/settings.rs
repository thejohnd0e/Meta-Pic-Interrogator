use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::domain::{AppError, AppResult, Preset, SettingsDocument};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SettingsFile {
    pub schema_version: u32,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    pub preset_id: Option<String>,
    pub presets: Vec<Preset>,
}

impl Default for SettingsFile {
    fn default() -> Self {
        Self {
            schema_version: 1,
            provider_id: None,
            model_id: None,
            preset_id: None,
            presets: vec![
                Preset {
                    id: "concise".to_owned(),
                    name: "Concise".to_owned(),
                    prompt: "Describe the image clearly and briefly.".to_owned(),
                },
                Preset {
                    id: "detailed".to_owned(),
                    name: "Detailed".to_owned(),
                    prompt: "Describe the image with useful visual detail.".to_owned(),
                },
            ],
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

    pub fn validate_preset(preset: &Preset) -> AppResult<()> {
        if preset.name.trim().is_empty() || preset.prompt.trim().is_empty() {
            return Err(AppError::MalformedResponse(
                "preset name and prompt are required".to_owned(),
            ));
        }
        Ok(())
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
    fn seeds_editable_presets_only_on_first_load() {
        let settings = SettingsFile::default();
        assert_eq!(settings.schema_version, 1);
        assert!(settings.presets.iter().any(|preset| preset.id == "concise"));
    }

    #[test]
    fn rejects_empty_preset_fields() {
        let preset = Preset {
            id: "x".to_owned(),
            name: String::new(),
            prompt: "prompt".to_owned(),
        };
        assert!(SettingsStore::validate_preset(&preset).is_err());
    }
}
