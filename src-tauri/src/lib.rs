pub mod auth_status;
pub mod claude_settings;
pub mod config;
pub mod controller;
pub mod env_builder;
pub mod log_buffer;
pub mod platform;
pub mod proc_guard;
pub mod shutdown;
pub mod supervisor;
pub mod tray;

use auth_status::{AuthState, inspect_auth};
use config::AppConfig;
use controller::{Controller, StatusDto};
use serde::Serialize;
use std::{path::PathBuf, sync::Arc};
use tauri::{AppHandle, Manager, State, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_clipboard_manager::ClipboardExt;

type Ctl<'a> = State<'a, Arc<Controller>>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvReport {
    codex_cli: Option<String>,
    auth_path: String,
    auth: AuthState,
    claude_settings: String,
    settings_ok: bool,
    settings_error: Option<String>,
    system_proxy: Option<String>,
}

#[tauri::command]
fn get_status(ctl: Ctl) -> StatusDto {
    ctl.status()
}

#[tauri::command]
fn set_models(ctl: Ctl, models: Option<Vec<String>>) {
    ctl.set_models(models);
}

#[tauri::command]
fn set_takeover(ctl: Ctl, on: bool) {
    ctl.set_takeover(on);
}

#[tauri::command]
fn restart_proxy(ctl: Ctl) {
    ctl.sup.restart();
}

#[tauri::command]
fn kill_port_owner(ctl: Ctl) {
    let c = ctl.inner().clone();
    std::thread::spawn(move || c.sup.kill_port_owner_and_start());
}

#[tauri::command]
fn save_config(app: AppHandle, ctl: Ctl, cfg: AppConfig) {
    apply_autostart(&app, cfg.autostart);
    ctl.save_config(cfg);
}

#[tauri::command]
fn open_codex_login() -> Result<(), String> {
    platform::open_codex_login()
}

#[tauri::command]
async fn refresh_models(ctl: Ctl<'_>) -> Result<Vec<String>, String> {
    let c = ctl.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        c.sup.refresh_models();
        c.notify();
        c.sup.models()
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn copy_logs(app: AppHandle, ctl: Ctl) -> Result<usize, String> {
    let logs = ctl.sup.logs();
    app.clipboard().write_text(logs.join("\n")).map_err(|e| e.to_string())?;
    Ok(logs.len())
}

#[tauri::command]
async fn detect_environment(ctl: Ctl<'_>) -> Result<EnvReport, String> {
    let c = ctl.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let auth_path = c.auth_path();
        let settings = c.settings_path();
        let parsed = claude_settings::read_settings(&settings);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        EnvReport {
            codex_cli: platform::codex_cli_version(),
            auth: inspect_auth(&auth_path, now),
            auth_path: auth_path.to_string_lossy().into_owned(),
            claude_settings: settings.to_string_lossy().into_owned(),
            settings_ok: parsed.is_ok(),
            settings_error: parsed.err().map(|e| e.to_string()),
            system_proxy: platform::system_proxy(),
        }
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn test_outbound(proxy: Option<String>) -> Result<u64, String> {
    tauri::async_runtime::spawn_blocking(move || platform::test_outbound(proxy.as_deref())).await.map_err(|e| e.to_string())?
}

#[tauri::command]
fn preview_start(ctl: Ctl, cfg: AppConfig) {
    ctl.preview_start(cfg);
}

#[tauri::command]
fn finish_onboarding(app: AppHandle, ctl: Ctl, cfg: AppConfig) {
    apply_autostart(&app, cfg.autostart);
    ctl.finish_onboarding(cfg);
}

#[tauri::command]
fn hide_window(app: AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
}

pub fn apply_autostart(app: &AppHandle, on: bool) {
    // 开发构建不注册开机自启，免得把 target/debug 里的程序挂进系统
    if config::DEV_MODE {
        return;
    }
    let al = app.autolaunch();
    let _ = if on { al.enable() } else { al.disable() };
}

pub fn quit(app: &AppHandle, ctl: &Arc<Controller>) {
    ctl.shutdown();
    app.exit(0);
}

fn sidecar_path() -> PathBuf {
    let exe = std::env::current_exe().expect("current exe");
    let name = if cfg!(windows) { "claude-codex.exe" } else { "claude-codex" };
    exe.parent().expect("exe dir").join(name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let hidden = std::env::args().any(|a| a == "--hidden");
    let mut builder = tauri::Builder::default();
    // 开发构建不注册单实例，避免与已安装的正式版互相抢占
    if !config::DEV_MODE {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| tray::show_main(app)));
    }
    builder
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--hidden"])))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(move |app| {
            let handle = app.handle().clone();
            let home = dirs::home_dir().expect("home dir");
            let data_dir = app.path().app_data_dir()?.join(if config::DEV_MODE { "dev" } else { "" });
            let ctl = Controller::new(handle.clone(), sidecar_path(), home, data_dir);
            let tray = tray::build(&handle, ctl.clone())?;
            ctl.set_status_listener(move |st| tray.update(st));
            if ctl.onboarded() {
                if ctl.config().autostart != handle.autolaunch().is_enabled().unwrap_or(false) {
                    apply_autostart(&handle, ctl.config().autostart);
                }
                ctl.sup.start();
            }
            let window = app.get_webview_window("main").expect("main window");
            shutdown::install(ctl.clone(), &window);
            if !hidden || !ctl.onboarded() {
                tray::show_main(&handle);
            }
            app.manage(ctl.clone());
            ctl.notify();
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_status,
            set_takeover,
            set_models,
            restart_proxy,
            kill_port_owner,
            save_config,
            open_codex_login,
            refresh_models,
            copy_logs,
            detect_environment,
            test_outbound,
            preview_start,
            finish_onboarding,
            hide_window
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, code, .. } = &event {
                // 托盘「退出」走 app.exit(0) → code=Some(0)；其它（窗口全关）不退出
                if code.is_none() {
                    api.prevent_exit();
                }
            }
            if let tauri::RunEvent::Exit = event {
                if let Some(c) = app.try_state::<Arc<Controller>>() {
                    c.shutdown();
                }
            }
        });
}
