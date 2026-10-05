mod commands;
pub mod credentials;
pub mod domain;
pub mod image;
pub mod metadata;
pub mod settings;

#[tauri::command]
fn health() -> &'static str {
    "ok"
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .invoke_handler(tauri::generate_handler![
            health,
            commands::inspect_image,
            commands::describe_image,
            commands::cancel_description,
            commands::list_presets,
            commands::create_preset,
            commands::update_preset,
            commands::delete_preset,
            commands::provider_status,
            commands::refresh_models,
            commands::set_credential,
            commands::delete_credential,
            commands::save_png_copy
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::health;

    #[test]
    fn health_command_reports_ready() {
        assert_eq!(health(), "ok");
    }
}
