use crate::domain::{
    AppError, AppResult, DescriptionDraft, ImageInfo, InputImage, Preset, ProviderConfig,
    SaveRequest, SettingsDocument, VisionCapabilities, VisionModel,
};
use crate::providers::{VisionBackend, VisionRequest};
use crate::{image, metadata};
use serde::Serialize;
use tauri::{Emitter, Manager};

use std::sync::{
    atomic::{AtomicU64, Ordering},
    OnceLock,
};

#[cfg(not(windows))]
use std::sync::Mutex;

#[cfg(not(windows))]
static MEMORY_CREDENTIALS: OnceLock<Mutex<crate::credentials::MemoryCredentialStore>> =
    OnceLock::new();

static REQUEST_REGISTRY: OnceLock<crate::providers::RequestRegistry> = OnceLock::new();
static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Serialize)]
struct DescriptionDelta {
    request_id: u64,
    text: String,
}

#[derive(Clone, Serialize)]
struct DescriptionStarted {
    request_id: u64,
}

fn settings_store(app: &tauri::AppHandle) -> AppResult<crate::settings::SettingsStore> {
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|error| AppError::LocalMetadata(error.to_string()))?;
    Ok(crate::settings::SettingsStore::new(
        directory.join("settings.json"),
    ))
}

fn request_registry() -> &'static crate::providers::RequestRegistry {
    REQUEST_REGISTRY.get_or_init(Default::default)
}

fn credential_secret(provider_id: &str) -> AppResult<String> {
    use crate::credentials::CredentialStore;

    #[cfg(windows)]
    {
        let store = crate::credentials::WindowsCredentialStore;
        store.get(provider_id)?.ok_or_else(|| {
            AppError::Authentication("provider credential is not configured".to_owned())
        })
    }

    #[cfg(not(windows))]
    {
        let store = MEMORY_CREDENTIALS.get_or_init(|| Mutex::new(Default::default()));
        let store = store
            .lock()
            .map_err(|_| AppError::Authentication("credential store unavailable".to_owned()))?;
        store.get(provider_id)?.ok_or_else(|| {
            AppError::Authentication("provider credential is not configured".to_owned())
        })
    }
}

fn with_credential_store<R>(
    f: impl FnOnce(&mut dyn crate::credentials::CredentialStore) -> AppResult<R>,
) -> AppResult<R> {
    #[cfg(windows)]
    {
        f(&mut crate::credentials::WindowsCredentialStore)
    }

    #[cfg(not(windows))]
    {
        let store = MEMORY_CREDENTIALS.get_or_init(|| Mutex::new(Default::default()));
        let mut store = store
            .lock()
            .map_err(|_| AppError::Authentication("credential store unavailable".to_owned()))?;
        f(&mut *store)
    }
}

fn describe_with_backend(
    input: InputImage,
    _provider: ProviderConfig,
    preset: Preset,
    backend: &dyn VisionBackend,
    on_delta: &mut dyn FnMut(String),
) -> AppResult<DescriptionDraft> {
    if !backend.capabilities().image_input {
        return Err(AppError::NoVisionSupport(backend.provider_id()));
    }
    let source =
        std::fs::read(&input.path).map_err(|error| AppError::LocalImage(error.to_string()))?;
    let decoded = image::decode_supported(
        &source,
        image::ImageLimits::default(),
        image::Orientation::TopLeft,
    )?;
    let normalized = image::normalize_for_ai(&decoded, 4096, 10 * 1024 * 1024)?;
    let response = backend.describe_stream(
        VisionRequest {
            prompt: preset.prompt,
            mime: normalized.mime.to_owned(),
            image: normalized.bytes,
        },
        on_delta,
    )?;
    DescriptionDraft::new(response.text)
}

#[tauri::command]
pub fn inspect_image(input: InputImage) -> AppResult<ImageInfo> {
    let bytes =
        std::fs::read(&input.path).map_err(|error| AppError::LocalImage(error.to_string()))?;
    let decoded = image::decode_supported(
        &bytes,
        image::ImageLimits::default(),
        image::Orientation::TopLeft,
    )?;
    Ok(ImageInfo {
        path: input.path,
        format: format!("{:?}", decoded.format),
        width: decoded.width,
        height: decoded.height,
        has_alpha: decoded.image.color().has_alpha(),
    })
}

#[tauri::command]
pub fn describe_image(
    input: InputImage,
    provider: ProviderConfig,
    preset: Preset,
    window: tauri::Window,
) -> AppResult<DescriptionDraft> {
    let registry = request_registry();
    let _guard = registry.begin()?;
    let request_id = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    window
        .emit("description-started", DescriptionStarted { request_id })
        .map_err(|_| AppError::Network("description event unavailable".to_owned()))?;
    if registry.is_cancelled()? {
        return Err(AppError::Cancellation("description cancelled".to_owned()));
    }
    let backend: Box<dyn VisionBackend> = if provider.provider_id == crate::chatgpt::PROVIDER_ID {
        let token = with_credential_store(|store| crate::chatgpt::access_token(store))?;
        Box::new(crate::providers::ApiProvider::new_with_model(
            provider.provider_id.clone(),
            crate::chatgpt::ResponsesTransport::new(&token, &provider.model_id)?,
            provider.model_id.clone(),
        ))
    } else {
        let secret = credential_secret(&provider.provider_id)?;
        crate::providers::build_openai_backend(&provider, &secret)?
    };
    let result = describe_with_backend(input, provider, preset, backend.as_ref(), &mut |text| {
        let _ = window.emit("description-delta", DescriptionDelta { request_id, text });
    });
    if registry.is_cancelled()? {
        return Err(AppError::Cancellation("description cancelled".to_owned()));
    }
    result
}

#[tauri::command]
pub fn chatgpt_status() -> AppResult<crate::chatgpt::ChatGptStatus> {
    with_credential_store(|store| crate::chatgpt::status(store))
}

#[tauri::command]
pub async fn chatgpt_sign_in() -> AppResult<crate::chatgpt::ChatGptStatus> {
    tauri::async_runtime::spawn_blocking(|| {
        with_credential_store(|store| {
            crate::chatgpt::sign_in(store, crate::chatgpt::open_in_browser)
        })
    })
    .await
    .map_err(|_| AppError::Network("ChatGPT sign-in stopped unexpectedly".to_owned()))?
}

#[tauri::command]
pub fn chatgpt_cancel_sign_in() {
    crate::chatgpt::cancel_sign_in();
}

#[tauri::command]
pub async fn chatgpt_sign_out() -> AppResult<()> {
    tauri::async_runtime::spawn_blocking(|| {
        with_credential_store(|store| crate::chatgpt::sign_out(store))
    })
    .await
    .map_err(|_| AppError::Network("ChatGPT sign-out stopped unexpectedly".to_owned()))?
}

#[tauri::command]
pub async fn chatgpt_models() -> AppResult<Vec<VisionModel>> {
    tauri::async_runtime::spawn_blocking(|| {
        let token = with_credential_store(|store| crate::chatgpt::access_token(store))?;
        crate::chatgpt::list_models(&token)
    })
    .await
    .map_err(|_| AppError::Network("ChatGPT model list stopped unexpectedly".to_owned()))?
}

#[tauri::command]
pub fn cancel_description() -> AppResult<()> {
    request_registry().cancel()
}

#[tauri::command]
pub fn list_presets(app: tauri::AppHandle) -> AppResult<Vec<Preset>> {
    settings_store(&app)?.list_presets()
}

#[tauri::command]
pub fn load_settings(app: tauri::AppHandle) -> AppResult<SettingsDocument> {
    let settings = settings_store(&app)?.load()?;
    Ok(SettingsDocument {
        proxy: settings.proxy,
        schema_version: settings.schema_version,
        provider_id: settings.provider_id,
        model_id: settings.model_id,
        endpoint: settings.endpoint,
        preset_id: settings.preset_id,
    })
}

#[tauri::command]
pub fn save_settings(app: tauri::AppHandle, settings: SettingsDocument) -> AppResult<()> {
    if settings.provider_id.as_deref().is_some_and(str::is_empty)
        || settings.model_id.as_deref().is_some_and(str::is_empty)
        || settings.endpoint.as_deref().is_some_and(str::is_empty)
    {
        return Err(AppError::MalformedResponse(
            "settings values cannot be empty".to_owned(),
        ));
    }
    let store = settings_store(&app)?;
    let mut current = store.load()?;
    current.schema_version = 1;
    current.provider_id = settings.provider_id;
    current.model_id = settings.model_id;
    current.endpoint = settings.endpoint;
    current.preset_id = settings.preset_id;
    store.save(&current)
}

/// Rebuilds the process-wide proxy from stored settings and credential.
pub fn apply_stored_proxy(app: &tauri::AppHandle) -> AppResult<()> {
    let proxy = settings_store(app)?.load()?.proxy.unwrap_or_default();
    let password = with_credential_store(|store| store.get(crate::network::PASSWORD_KEY))?;
    crate::network::apply(crate::network::proxy_url(&proxy, password.as_deref())?);
    Ok(())
}

#[tauri::command]
pub fn save_proxy(
    app: tauri::AppHandle,
    proxy: crate::network::ProxySettings,
) -> AppResult<crate::network::ProxySettings> {
    let proxy = crate::network::normalize(&proxy)?;
    // Validate before persisting so a bad value never reaches settings.json.
    crate::network::proxy_url(&proxy, None)?;
    let store = settings_store(&app)?;
    let mut current = store.load()?;
    current.proxy = Some(proxy.clone());
    store.save(&current)?;
    apply_stored_proxy(&app)?;
    Ok(proxy)
}

#[tauri::command]
pub fn create_preset(app: tauri::AppHandle, preset: Preset) -> AppResult<Preset> {
    settings_store(&app)?.create_preset(preset)
}

#[tauri::command]
pub fn update_preset(app: tauri::AppHandle, preset: Preset) -> AppResult<Preset> {
    settings_store(&app)?.update_preset(preset)
}

#[tauri::command]
pub fn delete_preset(app: tauri::AppHandle, preset_id: String) -> AppResult<()> {
    settings_store(&app)?.delete_preset(&preset_id)
}

#[tauri::command]
pub fn provider_status(provider_id: String) -> AppResult<VisionCapabilities> {
    Ok(crate::providers::provider_capabilities(&provider_id))
}

#[tauri::command]
pub fn credential_status(provider_id: String) -> AppResult<bool> {
    use crate::credentials::CredentialStore;

    #[cfg(windows)]
    {
        let store = crate::credentials::WindowsCredentialStore;
        Ok(store.get(&provider_id)?.is_some())
    }

    #[cfg(not(windows))]
    {
        let store = MEMORY_CREDENTIALS
            .get_or_init(|| Mutex::new(Default::default()))
            .lock()
            .map_err(|_| AppError::Authentication("credential store unavailable".to_owned()))?;
        Ok(store.get(&provider_id)?.is_some())
    }
}

#[tauri::command]
pub fn refresh_models(provider: ProviderConfig) -> AppResult<Vec<VisionModel>> {
    let models = crate::providers::configured_models(&provider)?;
    credential_secret(&provider.provider_id)?;
    Ok(models)
}

#[tauri::command]
pub fn set_credential(provider_id: String, secret: String) -> AppResult<()> {
    use crate::credentials::CredentialStore;

    #[cfg(windows)]
    {
        let mut store = crate::credentials::WindowsCredentialStore;
        store.set(&provider_id, &secret)
    }

    #[cfg(not(windows))]
    {
        let store = MEMORY_CREDENTIALS.get_or_init(|| Mutex::new(Default::default()));
        let mut store = store
            .lock()
            .map_err(|_| AppError::Authentication("credential store unavailable".to_owned()))?;
        store.set(&provider_id, &secret)
    }
}

#[tauri::command]
pub fn delete_credential(provider_id: String) -> AppResult<()> {
    use crate::credentials::CredentialStore;

    #[cfg(windows)]
    {
        let mut store = crate::credentials::WindowsCredentialStore;
        store.delete(&provider_id)
    }

    #[cfg(not(windows))]
    {
        let store = MEMORY_CREDENTIALS.get_or_init(|| Mutex::new(Default::default()));
        let mut store = store
            .lock()
            .map_err(|_| AppError::Authentication("credential store unavailable".to_owned()))?;
        store.delete(&provider_id)
    }
}

#[tauri::command]
pub fn save_png_copy(request: SaveRequest) -> AppResult<String> {
    let source = std::fs::read(&request.source_path)
        .map_err(|error| AppError::LocalImage(error.to_string()))?;
    let decoded = image::decode_supported(
        &source,
        image::ImageLimits::default(),
        image::Orientation::TopLeft,
    )?;
    let clean = image::encode_clean_png(&decoded.image)?;
    let output = metadata::write_metadata_png(&clean, &request.description, &request.provenance)?;
    metadata::save_verified_png(std::path::Path::new(&request.destination_path), &output)?;
    Ok(request.destination_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{ApiProvider, MockTransport};
    use ::image::{DynamicImage, Rgba, RgbaImage};

    #[test]
    fn describe_flow_normalizes_source_and_returns_provider_text() {
        let path =
            std::env::temp_dir().join(format!("metapic-describe-{}.png", std::process::id()));
        let image = RgbaImage::from_pixel(2, 1, Rgba([12, 34, 56, 255]));
        DynamicImage::ImageRgba8(image)
            .save(&path)
            .expect("fixture saves");
        let source = std::fs::read(&path).expect("fixture reads");
        let backend = ApiProvider::new(
            "openai",
            MockTransport {
                status: 200,
                body: "A small blue image.".to_owned(),
            },
            Vec::new(),
        );
        let draft = describe_with_backend(
            InputImage {
                path: path.to_string_lossy().into_owned(),
            },
            ProviderConfig {
                provider_id: "openai".to_owned(),
                model_id: "vision".to_owned(),
                endpoint: None,
            },
            Preset {
                id: "concise".to_owned(),
                name: "Concise".to_owned(),
                prompt: "Describe briefly".to_owned(),
            },
            &backend,
            &mut |_| {},
        )
        .expect("description succeeds");
        assert_eq!(draft.text, "A small blue image.");
        assert!(!draft.is_dirty);
        assert_eq!(std::fs::read(&path).expect("source rereads"), source);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn provider_status_does_not_enable_unverified_vision() {
        assert!(
            provider_status("openai".to_owned())
                .expect("status succeeds")
                .image_input
        );
        assert!(
            !provider_status("supergrok".to_owned())
                .expect("status succeeds")
                .image_input
        );
    }
}
