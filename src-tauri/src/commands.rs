use crate::domain::{
    AppError, AppResult, DescriptionDraft, ImageInfo, InputImage, Preset, ProviderConfig,
    SaveRequest, VisionCapabilities, VisionModel,
};
use crate::{image, metadata};

fn pending<T>(name: &str) -> AppResult<T> {
    Err(AppError::NotImplemented(name.to_owned()))
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
