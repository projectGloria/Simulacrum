//! Simulacrum: shape images and export multi-size .ico files.
//! The image modules are `pub` so the example CLI and integration tests use the exact same
//! pipeline as the app.

pub mod android;
mod commands;
pub mod error;
pub mod folder_icon;
pub mod icns;
pub mod ico;
pub mod mask;
pub mod naming;
pub mod pipeline;
pub mod project;
pub mod resample;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(commands::Shared::default())
        .invoke_handler(tauri::generate_handler![
            commands::load_images,
            commands::load_clipboard_image,
            commands::remove_image,
            commands::set_size_override,
            commands::preview,
            commands::render_png_bytes,
            commands::export_single,
            commands::export_icns,
            commands::export_pack,
            commands::apply_folder_icon,
            commands::set_android_mono,
            commands::android_preview,
            commands::export_android,
            commands::scan_project,
            commands::export_to_project,
            commands::export_batch,
            commands::cancel_batch,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start Simulacrum");
}
