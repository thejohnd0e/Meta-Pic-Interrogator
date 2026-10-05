use crate::domain::{
    AppError, AppResult, DescriptionDraft, ImageInfo, InputImage, Preset, ProviderConfig,
    SaveRequest, VisionCapabilities, VisionModel,
};

fn pending<T>(name: &str) -> AppResult<T> {
    Err(AppError::NotImplemented(name.to_owned()))
}

#[tauri::command]
pub fn inspect_image(_input: InputImage) -> AppResult<ImageInfo> {
    pending("inspect_image")
}

#[tauri::command]
pub fn describe_image(
    _input: InputImage,
    _provider: ProviderConfig,
    _preset: Preset,
) -> AppResult<DescriptionDraft> {
    pending("describe_image")
}

#[tauri::command]
pub fn cancel_description() -> AppResult<()> {
    pending("cancel_description")
}

#[tauri::command]
pub fn list_presets() -> AppResult<Vec<Preset>> {
    pending("list_presets")
}

#[tauri::command]
pub fn create_preset(_preset: Preset) -> AppResult<Preset> {
    pending("create_preset")
}

#[tauri::command]
pub fn update_preset(_preset: Preset) -> AppResult<Preset> {
    pending("update_preset")
}

#[tauri::command]
pub fn delete_preset(_preset_id: String) -> AppResult<()> {
    pending("delete_preset")
}

#[tauri::command]
pub fn provider_status(_provider_id: String) -> AppResult<VisionCapabilities> {
    pending("provider_status")
}

#[tauri::command]
pub fn refresh_models(_provider: ProviderConfig) -> AppResult<Vec<VisionModel>> {
    pending("refresh_models")
}

#[tauri::command]
pub fn set_credential(_provider_id: String, _secret: String) -> AppResult<()> {
    pending("set_credential")
}

#[tauri::command]
pub fn delete_credential(_provider_id: String) -> AppResult<()> {
    pending("delete_credential")
}

#[tauri::command]
pub fn save_png_copy(_request: SaveRequest) -> AppResult<String> {
    pending("save_png_copy")
}
