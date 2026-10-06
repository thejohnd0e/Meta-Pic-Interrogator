//! Prompt presets stored as one Markdown file each: `<app data>/presets/<id>.md`.
//!
//! A file written by the app starts with a small front matter block holding the
//! display name; everything after it is the prompt. Files without front matter
//! (for example hand-written or imported ones) are read as a plain prompt named
//! after the file.

use std::path::{Path, PathBuf};

use crate::domain::{AppError, AppResult, Preset};
use crate::settings::SettingsStore;

/// Largest prompt file the app will read or write.
pub const MAX_PROMPT_BYTES: u64 = 1024 * 1024;
const MAX_IMPORT_FILES: usize = 200;
const MAX_ID_CHARS: usize = 60;

pub struct PresetStore {
    dir: PathBuf,
}

pub fn default_presets() -> Vec<Preset> {
    [
        (
            "concise",
            "Concise",
            "Describe the image clearly and briefly.",
        ),
        (
            "detailed",
            "Detailed",
            "Describe the image with useful visual detail.",
        ),
        (
            "appearance",
            "Appearance",
            "Focus on visible appearance, colors, and visual style.",
        ),
        (
            "clothing",
            "Clothing",
            "Describe clothing, accessories, and materials.",
        ),
        (
            "composition",
            "Composition",
            "Describe composition, framing, layout, and spatial relationships.",
        ),
        (
            "photography",
            "Photography",
            "Describe photographic qualities, lighting, depth, and perspective.",
        ),
    ]
    .into_iter()
    .map(|(id, name, prompt)| Preset {
        id: id.to_owned(),
        name: name.to_owned(),
        prompt: prompt.to_owned(),
    })
    .collect()
}

fn io_error(error: std::io::Error) -> AppError {
    AppError::LocalMetadata(error.to_string())
}

fn invalid(message: &str) -> AppError {
    AppError::MalformedResponse(message.to_owned())
}

/// Ids become file names, so only letters, digits, `-` and `_` are allowed.
pub fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.chars().count() <= MAX_ID_CHARS
        && id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        && !is_reserved_name(id)
}

fn is_reserved_name(id: &str) -> bool {
    let lower = id.to_lowercase();
    matches!(lower.as_str(), "con" | "prn" | "aux" | "nul")
        || ["com", "lpt"].iter().any(|prefix| {
            lower
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.len() == 1 && rest.chars().all(|c| c.is_ascii_digit()))
        })
}

fn slugify(name: &str) -> String {
    let mut slug = String::new();
    for c in name.trim().chars() {
        if c.is_alphanumeric() {
            slug.extend(c.to_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_matches('-').chars().take(MAX_ID_CHARS).collect();
    let slug = slug.trim_matches('-').to_owned();
    if slug.is_empty() || is_reserved_name(&slug) {
        format!("{slug}-preset").trim_start_matches('-').to_owned()
    } else {
        slug
    }
}

fn clean_name(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Splits file text into `(name, prompt)`. The name is `None` without front matter.
pub fn parse_file(text: &str) -> (Option<String>, String) {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.split_inclusive('\n');
    if lines.next().map(str::trim_end) != Some("---") {
        return (None, text.to_owned());
    }
    let mut name = None;
    let mut consumed = text.split_inclusive('\n').next().map_or(0, str::len);
    for line in lines {
        consumed += line.len();
        let trimmed = line.trim_end();
        if trimmed == "---" {
            let body = text[consumed..].trim_start_matches(['\r', '\n']);
            return (name, body.to_owned());
        }
        if let Some(value) = trimmed.strip_prefix("name:") {
            let value = value.trim();
            name = Some(serde_json::from_str::<String>(value).unwrap_or_else(|_| value.to_owned()));
        }
    }
    // No closing marker: it was not front matter after all.
    (None, text.to_owned())
}

pub fn render_file(preset: &Preset) -> String {
    let name = serde_json::to_string(&clean_name(&preset.name)).unwrap_or_default();
    format!("---\nname: {name}\n---\n{}", preset.prompt)
}

fn validate(preset: &Preset) -> AppResult<()> {
    if clean_name(&preset.name).is_empty() || preset.prompt.trim().is_empty() {
        return Err(invalid("preset name and prompt are required"));
    }
    if preset.prompt.len() as u64 > MAX_PROMPT_BYTES {
        return Err(invalid("preset prompt is too large (limit 1 MB)"));
    }
    Ok(())
}

impl PresetStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn file(&self, id: &str) -> AppResult<PathBuf> {
        if !is_valid_id(id) {
            return Err(invalid("invalid preset id"));
        }
        Ok(self.dir.join(format!("{id}.md")))
    }

    fn write_atomic(path: &Path, text: &str) -> AppResult<()> {
        let temp = path.with_extension("md.tmp");
        std::fs::write(&temp, text).map_err(io_error)?;
        std::fs::rename(&temp, path).map_err(|error| {
            let _ = std::fs::remove_file(&temp);
            io_error(error)
        })
    }

    /// Creates the presets folder on first use, moving presets that older
    /// versions kept in `settings.json` (or the built-in defaults) into files.
    pub fn initialize(&self, settings: &SettingsStore) -> AppResult<()> {
        if self.dir.exists() {
            return Ok(());
        }
        let mut file = settings.load()?;
        let migrated_legacy = !file.presets.is_empty();
        let seed = if migrated_legacy {
            std::mem::take(&mut file.presets)
        } else {
            default_presets()
        };
        let staging = self.dir.with_extension("migrating");
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging).map_err(io_error)?;
        let staged = PresetStore::new(&staging);
        for mut preset in seed {
            if validate(&preset).is_err() {
                continue;
            }
            if !is_valid_id(&preset.id) {
                preset.id = slugify(&preset.name);
            }
            staged.create_unique(preset)?;
        }
        std::fs::rename(&staging, &self.dir).map_err(io_error)?;
        if migrated_legacy {
            settings.save(&file)?;
        }
        Ok(())
    }

    pub fn list(&self) -> AppResult<Vec<Preset>> {
        let mut presets = Vec::new();
        for entry in std::fs::read_dir(&self.dir).map_err(io_error)? {
            let path = entry.map_err(io_error)?.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if !is_valid_id(id) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let (name, prompt) = parse_file(&text);
            presets.push(Preset {
                id: id.to_owned(),
                name: name
                    .map(|name| clean_name(&name))
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| id.to_owned()),
                prompt,
            });
        }
        presets.sort_by_key(|preset| (preset.name.to_lowercase(), preset.id.clone()));
        Ok(presets)
    }

    /// Writes a new preset under a free id derived from `preset.id` (or its name).
    fn create_unique(&self, preset: Preset) -> AppResult<Preset> {
        validate(&preset)?;
        let base = if is_valid_id(&preset.id) {
            preset.id.clone()
        } else {
            slugify(&preset.name)
        };
        for attempt in 1..1000 {
            let id = match attempt {
                1 => base.clone(),
                n => {
                    let suffix = format!("-{n}");
                    let room = MAX_ID_CHARS - suffix.chars().count();
                    format!("{}{suffix}", base.chars().take(room).collect::<String>())
                }
            };
            let path = self.file(&id)?;
            if path.exists() {
                continue;
            }
            let created = Preset {
                id,
                name: clean_name(&preset.name),
                prompt: preset.prompt.clone(),
            };
            Self::write_atomic(&path, &render_file(&created))?;
            return Ok(created);
        }
        Err(invalid("could not find a free preset id"))
    }

    pub fn create(&self, preset: Preset) -> AppResult<Preset> {
        // The caller never picks the file name: derive it from the name.
        self.create_unique(Preset {
            id: String::new(),
            ..preset
        })
    }

    pub fn update(&self, preset: Preset) -> AppResult<Preset> {
        validate(&preset)?;
        let path = self.file(&preset.id)?;
        if !path.exists() {
            return Err(invalid("preset not found"));
        }
        let saved = Preset {
            name: clean_name(&preset.name),
            ..preset
        };
        Self::write_atomic(&path, &render_file(&saved))?;
        Ok(saved)
    }

    pub fn delete(&self, id: &str) -> AppResult<()> {
        let path = self.file(id)?;
        if !path.exists() {
            return Err(invalid("preset not found"));
        }
        std::fs::remove_file(path).map_err(io_error)
    }

    /// Imports `.md`/`.markdown`/`.txt` files as new presets. Unreadable,
    /// oversized, empty or non-UTF-8 files are skipped.
    pub fn import(&self, paths: &[String]) -> AppResult<Vec<Preset>> {
        let mut imported = Vec::new();
        for path in paths.iter().take(MAX_IMPORT_FILES) {
            let path = Path::new(path);
            let extension = path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(str::to_lowercase);
            if !matches!(extension.as_deref(), Some("md" | "markdown" | "txt")) {
                continue;
            }
            let Ok(metadata) = std::fs::metadata(path) else {
                continue;
            };
            if !metadata.is_file() || metadata.len() > MAX_PROMPT_BYTES {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(path) else {
                continue;
            };
            let (name, prompt) = parse_file(&text);
            let stem = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("Imported preset");
            let name = name
                .map(|name| clean_name(&name))
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| clean_name(stem));
            let preset = Preset {
                id: String::new(),
                name,
                prompt,
            };
            if let Ok(created) = self.create_unique(preset) {
                imported.push(created);
            }
        }
        if imported.is_empty() {
            return Err(invalid("no importable .md or .txt files were found"));
        }
        Ok(imported)
    }

    /// Copies a preset's file to `destination`.
    pub fn export(&self, id: &str, destination: &Path) -> AppResult<()> {
        let source = self.file(id)?;
        if !source.exists() {
            return Err(invalid("preset not found"));
        }
        std::fs::copy(source, destination)
            .map(|_| ())
            .map_err(io_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("metapic-presets-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp root");
        root
    }

    fn preset(name: &str, prompt: &str) -> Preset {
        Preset {
            id: String::new(),
            name: name.to_owned(),
            prompt: prompt.to_owned(),
        }
    }

    #[test]
    fn front_matter_round_trips_awkward_names_and_markdown_bodies() {
        let original = Preset {
            id: "x".to_owned(),
            name: "He said: \"hi\"".to_owned(),
            prompt: "# Title\n\n---\n\n- item\n".to_owned(),
        };
        let (name, prompt) = parse_file(&render_file(&original));
        assert_eq!(name.as_deref(), Some("He said: \"hi\""));
        assert_eq!(prompt, original.prompt);
    }

    #[test]
    fn files_without_front_matter_are_plain_prompts() {
        assert_eq!(
            parse_file("Just a prompt"),
            (None, "Just a prompt".to_owned())
        );
        assert_eq!(
            parse_file("---\nnot closed\ntext"),
            (None, "---\nnot closed\ntext".to_owned())
        );
        assert_eq!(
            parse_file("---\r\nname: A\r\n---\r\n\r\nBody\r\n"),
            (Some("A".to_owned()), "Body\r\n".to_owned())
        );
    }

    #[test]
    fn ids_are_safe_file_names() {
        assert!(is_valid_id("concise"));
        assert!(is_valid_id("описание-1"));
        for bad in ["", "..", "a/b", "a\\b", "a.b", "con", "COM1", "lpt9", "a b"] {
            assert!(!is_valid_id(bad), "{bad:?} should be rejected");
        }
        assert_eq!(slugify("My Prompt (v2)!"), "my-prompt-v2");
        assert!(is_valid_id(&slugify("???")));
        assert!(is_valid_id(&slugify("CON")));
    }

    #[test]
    fn crud_assigns_unique_ids_and_survives_reload() {
        let root = temp_root("crud");
        let store = PresetStore::new(root.join("presets"));
        std::fs::create_dir_all(store.dir()).expect("dir");

        let first = store.create(preset("Style Notes", "One")).expect("create");
        let second = store
            .create(preset("Style Notes", "Two"))
            .expect("create twin");
        assert_eq!(first.id, "style-notes");
        assert_eq!(second.id, "style-notes-2");

        let renamed = store
            .update(Preset {
                name: "Renamed".to_owned(),
                ..first.clone()
            })
            .expect("update");
        assert_eq!(renamed.id, first.id);
        let listed = store.list().expect("list");
        assert_eq!(
            listed.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            vec!["Renamed", "Style Notes"]
        );

        assert!(store
            .update(Preset {
                id: "ghost".to_owned(),
                ..renamed.clone()
            })
            .is_err());
        assert!(store.create(preset(" ", "x")).is_err());
        assert!(store.create(preset("n", "  ")).is_err());
        assert!(store
            .update(Preset {
                id: "../escape".to_owned(),
                ..renamed
            })
            .is_err());

        store.delete("style-notes").expect("delete");
        assert!(store.delete("style-notes").is_err());
        assert_eq!(store.list().expect("list").len(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn import_and_export_use_markdown_files() {
        let root = temp_root("io");
        let store = PresetStore::new(root.join("presets"));
        std::fs::create_dir_all(store.dir()).expect("dir");
        let source = root.join("Long Prompt.md");
        std::fs::write(&source, "# Heading\n\nDescribe everything.\n").expect("source");
        let skipped = root.join("image.png");
        std::fs::write(&skipped, "x").expect("png");

        let imported = store
            .import(&[
                source.to_string_lossy().into_owned(),
                skipped.to_string_lossy().into_owned(),
            ])
            .expect("import");
        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].name, "Long Prompt");
        assert_eq!(imported[0].prompt, "# Heading\n\nDescribe everything.\n");
        assert!(store
            .import(&[skipped.to_string_lossy().into_owned()])
            .is_err());

        let exported = root.join("out.md");
        store.export(&imported[0].id, &exported).expect("export");
        let (name, prompt) = parse_file(&std::fs::read_to_string(&exported).expect("read"));
        assert_eq!(name.as_deref(), Some("Long Prompt"));
        assert_eq!(prompt, imported[0].prompt);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn first_run_seeds_defaults_and_migrates_legacy_settings_once() {
        let root = temp_root("migrate");
        // Fresh install: defaults.
        let fresh = PresetStore::new(root.join("fresh").join("presets"));
        let fresh_settings = SettingsStore::new(root.join("fresh").join("settings.json"));
        fresh.initialize(&fresh_settings).expect("initialize");
        assert_eq!(fresh.list().expect("list").len(), default_presets().len());

        // Upgrade: presets saved in settings.json move into files and out of the JSON.
        let legacy_dir = root.join("legacy");
        std::fs::create_dir_all(&legacy_dir).expect("dir");
        let settings_path = legacy_dir.join("settings.json");
        std::fs::write(
            &settings_path,
            r#"{"schema_version":1,"provider_id":"openai","model_id":null,"preset_id":"mine",
                "presets":[{"id":"mine","name":"Mine","prompt":"My edited prompt"}]}"#,
        )
        .expect("legacy settings");
        let settings = SettingsStore::new(&settings_path);
        let store = PresetStore::new(legacy_dir.join("presets"));
        store.initialize(&settings).expect("migrate");
        let listed = store.list().expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].prompt, "My edited prompt");
        let raw = std::fs::read_to_string(&settings_path).expect("settings");
        assert!(!raw.contains("My edited prompt"));
        assert!(raw.contains("openai"));

        // A second run does nothing, even if the user deleted every preset.
        store.delete("mine").expect("delete");
        store.initialize(&settings).expect("second initialize");
        assert!(store.list().expect("list").is_empty());
        let _ = std::fs::remove_dir_all(root);
    }
}
