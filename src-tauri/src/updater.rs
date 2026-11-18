//! 自动更新：从 GitHub Release 的 latest.json 检查新版本（走客户端配置的对外代理）。
//! 正式版启动 30 秒后检查一次，之后每 30 分钟检查一次；发现新版本推给界面，并对每个新版本发一次系统通知。
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
const INTERVAL: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub current: String,
    pub notes: Option<String>,
    /// 正在下载 / 安装（界面重开后用来恢复「下载中」状态）
    pub installing: bool,
}

#[derive(Default)]
pub struct UpdaterState {
    pending: Mutex<Option<Update>>,
    notified: Mutex<Option<String>>,
    installing: AtomicBool,
}

impl UpdaterState {
    pub fn info(&self) -> Option<UpdateInfo> {
        let mut info = self.pending.lock().unwrap().as_ref().map(to_info)?;
        info.installing = self.is_installing();
        Some(info)
    }
}

fn to_info(u: &Update) -> UpdateInfo {
    UpdateInfo {
        version: u.version.clone(),
        current: u.current_version.clone(),
        notes: u.body.clone().filter(|s| !s.trim().is_empty()),
        installing: false,
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

/// 下载超时：安装包几 MB，慢网络下 30 秒不够
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// 正在安装时为 true；离开作用域（成功、出错、panic）一律复位，避免之后每次点击都返回 busy
struct InstallingGuard(AppHandle);

impl Drop for InstallingGuard {
    fn drop(&mut self) {
        self.0.state::<UpdaterState>().installing.store(false, Ordering::SeqCst);
    }
}

impl UpdaterState {
    pub fn is_installing(&self) -> bool {
        self.installing.load(Ordering::SeqCst)
    }
}

/// 下载并安装；进度通过 `update-progress`（0–100）推送。Windows 上安装程序启动后本进程会退出。
pub async fn install(app: AppHandle) -> Result<(), String> {
    if app.state::<UpdaterState>().installing.swap(true, Ordering::SeqCst) {
        return Err("busy".into());
    }
    let _guard = InstallingGuard(app.clone());
    install_inner(&app).await
}

async fn install_inner(app: &AppHandle) -> Result<(), String> {
    let pending = app.state::<UpdaterState>().pending.lock().unwrap().clone();
    let mut update = match pending {
        Some(u) => u,
        None => {
            check(app).await?;
            app.state::<UpdaterState>().pending.lock().unwrap().clone().ok_or("upToDate")?
        }
    };
    update.timeout = Some(DOWNLOAD_TIMEOUT);
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
    // 先撤销接管、停代理，再交给安装程序。
    // shutdown 里停代理用了 block_on，不能在异步任务里直接调用（会 panic：Cannot start a runtime from within a runtime），
    // 放到阻塞线程池里执行。
    let ctl = app.state::<Arc<Controller>>().inner().clone();
    tauri::async_runtime::spawn_blocking(move || ctl.shutdown()).await.map_err(|e| e.to_string())?;
    update.install(bytes).map_err(|e| e.to_string())?;
    app.restart();
}
