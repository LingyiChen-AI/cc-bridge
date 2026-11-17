//! 自动更新：从 GitHub Release 的 latest.json 检查新版本（走客户端配置的对外代理）。
//! 正式版启动 30 秒后检查一次，之后每 6 小时检查一次；发现新版本推给界面，并对每个新版本发一次系统通知。
//! 安装前先撤销接管、停掉代理，保证 settings.json 不会残留指向已退出进程的地址。
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::controller::Controller;
use crate::i18n::tr;

const FIRST_CHECK: Duration = Duration::from_secs(30);
const INTERVAL: Duration = Duration::from_secs(6 * 3600);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub current: String,
    pub notes: Option<String>,
}

#[derive(Default)]
pub struct UpdaterState {
    pending: Mutex<Option<Update>>,
    notified: Mutex<Option<String>>,
    installing: AtomicBool,
}

impl UpdaterState {
    pub fn info(&self) -> Option<UpdateInfo> {
        self.pending.lock().unwrap().as_ref().map(to_info)
    }
}

fn to_info(u: &Update) -> UpdateInfo {
    UpdateInfo {
        version: u.version.clone(),
        current: u.current_version.clone(),
        notes: u.body.clone().filter(|s| !s.trim().is_empty()),
    }
}

/// 检查一次；有新版本时记下并推送 `update-available` 事件。
pub async fn check(app: &AppHandle) -> Result<Option<UpdateInfo>, String> {
    let ctl = app.state::<Arc<Controller>>().inner().clone();
    let mut builder = app.updater_builder().timeout(Duration::from_secs(30));
    if let Some(p) = ctl.config().outbound() {
        let p = if p.contains("://") { p.to_string() } else { format!("http://{p}") };
        if let Ok(url) = tauri::Url::parse(&p) {
            builder = builder.proxy(url);
        }
    }
    let found = builder.build().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())?;
    let info = found.as_ref().map(to_info);
    *app.state::<UpdaterState>().pending.lock().unwrap() = found;
    let _ = app.emit("update-available", info.clone());
    Ok(info)
}

pub fn spawn_background(app: AppHandle) {
    if crate::config::DEV_MODE {
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            if let Ok(Some(info)) = check(&app).await {
                notify_once(&app, &info);
            }
            tokio::time::sleep(INTERVAL).await;
        }
    });
}

fn notify_once(app: &AppHandle, info: &UpdateInfo) {
    let st = app.state::<UpdaterState>();
    let mut last = st.notified.lock().unwrap();
    if last.as_deref() == Some(info.version.as_str()) {
        return;
    }
    *last = Some(info.version.clone());
    let lang = app.state::<Arc<Controller>>().config().lang();
    let _ = app
        .notification()
        .builder()
        .title(tr(lang, "notifyUpdateTitle").replace("{version}", &info.version))
        .body(tr(lang, "notifyUpdateBody"))
        .show();
}

/// 下载并安装；进度通过 `update-progress`（0–100）推送。Windows 上安装程序启动后本进程会退出。
pub async fn install(app: AppHandle) -> Result<(), String> {
    let st = app.state::<UpdaterState>();
    if st.installing.swap(true, Ordering::SeqCst) {
        return Err("busy".into());
    }
    let result = install_inner(&app).await;
    st.installing.store(false, Ordering::SeqCst);
    result
}

async fn install_inner(app: &AppHandle) -> Result<(), String> {
    let pending = app.state::<UpdaterState>().pending.lock().unwrap().clone();
    let update = match pending {
        Some(u) => u,
        None => {
            check(app).await?;
            app.state::<UpdaterState>().pending.lock().unwrap().clone().ok_or("upToDate")?
        }
    };
    let mut got: u64 = 0;
    let mut last_pct = u64::MAX;
    let bytes = update
        .download(
            |chunk, total| {
                got += chunk as u64;
                if let Some(total) = total.filter(|t| *t > 0) {
                    let pct = (got * 100 / total).min(100);
                    if pct != last_pct {
                        last_pct = pct;
                        let _ = app.emit("update-progress", pct);
                    }
                }
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;
    // 先撤销接管、停代理，再交给安装程序
    app.state::<Arc<Controller>>().shutdown();
    update.install(bytes).map_err(|e| e.to_string())?;
    app.restart();
}
