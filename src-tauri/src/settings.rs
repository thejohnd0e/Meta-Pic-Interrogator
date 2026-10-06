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
                Preset {
                    id: "appearance".to_owned(),
                    name: "Appearance".to_owned(),
                    prompt: "Focus on visible appearance, colors, and visual style.".to_owned(),
                },
                Preset {
                    id: "clothing".to_owned(),
                    name: "Clothing".to_owned(),
                    prompt: "Describe clothing, accessories, and materials.".to_owned(),
                },
                Preset {
                    id: "composition".to_owned(),
                    name: "Composition".to_owned(),
                    prompt: "Describe composition, framing, layout, and spatial relationships."
                        .to_owned(),
                },
                Preset {
                    id: "photography".to_owned(),
                    name: "Photography".to_owned(),
                    prompt: "Describe photographic qualities, lighting, depth, and perspective."
                        .to_owned(),
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

    pub fn list_presets(&self) -> AppResult<Vec<Preset>> {
        Ok(self.load()?.presets)
    }

    pub fn create_preset(&self, preset: Preset) -> AppResult<Preset> {
        Self::validate_preset(&preset)?;
        let mut settings = self.load()?;
        if settings.presets.iter().any(|item| item.id == preset.id) {
            return Err(AppError::MalformedResponse(
                "preset id already exists".to_owned(),
            ));
        }
        settings.presets.push(preset.clone());
        self.save(&settings)?;
        Ok(preset)
    }

    pub fn update_preset(&self, preset: Preset) -> AppResult<Preset> {
        Self::validate_preset(&preset)?;
        let mut settings = self.load()?;
        let existing = settings
            .presets
            .iter_mut()
            .find(|item| item.id == preset.id)
            .ok_or_else(|| AppError::MalformedResponse("preset not found".to_owned()))?;
        *existing = preset.clone();
        self.save(&settings)?;
        Ok(preset)
    }

    pub fn delete_preset(&self, preset_id: &str) -> AppResult<()> {
        let mut settings = self.load()?;
        let original_len = settings.presets.len();
        settings.presets.retain(|item| item.id != preset_id);
        if settings.presets.len() == original_len {
            return Err(AppError::MalformedResponse("preset not found".to_owned()));
        }
        self.save(&settings)
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
        assert_eq!(
            settings
                .presets
                .iter()
                .map(|preset| preset.id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "concise",
                "detailed",
                "appearance",
                "clothing",
                "composition",
                "photography"
            ]
        );
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

    #[test]
    fn persists_preset_crud_without_secrets() {
        let path =
            std::env::temp_dir().join(format!("metapic-settings-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let store = SettingsStore::new(&path);
        let preset = Preset {
            id: "custom".to_owned(),
            name: "Custom".to_owned(),
            prompt: "Describe texture".to_owned(),
        };

        assert!(store.list_presets().expect("initial load").len() >= 2);
        assert_eq!(store.create_preset(preset.clone()).expect("create"), preset);
        assert!(store.create_preset(preset.clone()).is_err());
        assert_eq!(
            store
                .update_preset(Preset {
                    prompt: "Describe texture and light".to_owned(),
                    ..preset.clone()
                })
                .expect("update")
                .prompt,
            "Describe texture and light"
        );
        store.delete_preset("custom").expect("delete");
        assert!(!store
            .list_presets()
            .expect("reload")
            .iter()
            .any(|item| item.id == "custom"));
        let raw = std::fs::read_to_string(&path).expect("settings file");
        assert!(!raw.contains("secret"));
        let _ = std::fs::remove_file(path);
    }
}
