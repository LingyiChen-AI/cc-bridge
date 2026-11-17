pub mod auth_status;
pub mod claude_settings;
pub mod config;
pub mod env_builder;
pub mod log_buffer;
pub mod proc_guard;
pub mod supervisor;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
