#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod capture;
mod ipc;

use ipc::commands::AppState;

fn main() {
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            ipc::commands::ping,
            ipc::commands::get_displays,
            ipc::commands::get_windows,
            ipc::commands::start_portal_picker,
            ipc::commands::get_audio_devices,
            ipc::commands::get_profiles,
            ipc::commands::save_profiles,
            ipc::commands::probe_encoders,
            ipc::commands::start_stream,
            ipc::commands::stop_stream,
            ipc::commands::get_status,
            ipc::commands::update_audio_mix,
            ipc::commands::get_vu,
            ipc::commands::copy_to_clipboard,
            ipc::commands::open_logs_dir,
        ])
        .run(tauri::generate_context!())
        .expect("error while running ezTopaz");
}
