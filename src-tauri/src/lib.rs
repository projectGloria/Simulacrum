//! Simulacrum: shape images and export multi-size .ico files.
//! The image modules are `pub` so the example CLI and integration tests use the exact same
//! pipeline as the app.

mod commands;
pub mod error;
pub mod ico;
pub mod mask;
pub mod naming;
pub mod pipeline;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(commands::Shared::default())
        .invoke_handler(tauri::generate_handler![
            commands::load_images,
            commands::remove_image,
            commands::preview,
            commands::export_single,
            commands::export_batch,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start Simulacrum");
}
