mod commands;
mod ecosystem;
mod error;
mod models;
mod osv;
mod registry;
mod scan;
mod source;
mod version;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(commands::AppState::new())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .invoke_handler(tauri::generate_handler![
            commands::scan_local,
            commands::scan_github,
            commands::cancel_scan,
            commands::export_report,
            commands::github_token_status,
            commands::save_github_token,
            commands::load_state,
            commands::save_state
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
