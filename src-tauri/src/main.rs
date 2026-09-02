#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod capture;
mod ipc;

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![ipc::commands::ping])
        .run(tauri::generate_context!())
        .expect("error while running ezTopaz");
}
