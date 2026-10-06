pub mod chatgpt;
mod commands;
pub mod credentials;
pub mod domain;
pub mod image;
pub mod metadata;
pub mod network;
pub mod oauth;
pub mod providers;
pub mod settings;
pub mod supergrok;

#[tauri::command]
fn health() -> &'static str {
    "ok"
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            // A broken proxy setting must not prevent the app from starting.
            let _ = commands::apply_stored_proxy(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            health,
            commands::inspect_image,
            commands::describe_image,
            commands::cancel_description,
            commands::chatgpt_status,
            commands::chatgpt_sign_in,
            commands::chatgpt_cancel_sign_in,
            commands::chatgpt_sign_out,
            commands::chatgpt_models,
            commands::supergrok_status,
            commands::supergrok_begin_sign_in,
            commands::supergrok_finish_sign_in,
            commands::supergrok_cancel_sign_in,
            commands::supergrok_sign_out,
            commands::supergrok_models,
            commands::list_presets,
            commands::load_settings,
            commands::save_settings,
            commands::save_proxy,
            commands::create_preset,
            commands::update_preset,
            commands::delete_preset,
            commands::provider_status,
            commands::credential_status,
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
