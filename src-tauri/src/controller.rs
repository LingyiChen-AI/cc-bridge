//! 应用编排：唯一会改 settings.json 的地方；汇总状态推给前端和托盘。
use crate::{
    auth_status::{AuthState, auth_file, inspect_auth},
    claude_settings::{SettingsError, apply_takeover, has_leftover, is_taken_over, read_settings, revoke_takeover, write_settings},
    config::{AppConfig, TakeoverRecord, load_json, save_json},
    custom_apis::{CustomApiDto, CustomApiInput, merge_edit, new_id, validate_key},
    proxy::custom_api::{self, AuthStyle, CustomApi, verify::{VerifyReport, verify}},
    supervisor::{LogHint, ProxyState, Supervisor},
};
use serde::Serialize;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}, mpsc},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StatusDto {
    pub proxy: ProxyState,
    pub port: u16,
    pub auth: AuthState,
    pub auth_path: String,
    pub auth_hint: Option<String>,
    pub network_hint: Option<String>,
    pub takeover: bool,
    pub taken_over: bool,
    pub settings_path: String,
    pub settings_error: Option<String>,
    pub models: Vec<String>,
    pub selected_models: Vec<String>,
    pub config: AppConfig,
    pub logs_tail: Vec<String>,
    pub onboarded: bool,
    pub dev_mode: bool,
    pub lang: String,
    pub custom_apis: Vec<CustomApiDto>,
}

/// 接管写入 settings.json 的模型：Codex 模型在前，自定义 API 在后。
pub(crate) fn takeover_models(selected_codex: Vec<String>, custom: Vec<String>) -> Vec<String> {
    let mut v = selected_codex;
    v.extend(custom);
    v
}

/// `scheme://host[:port]`，小写；用来判断编辑时是否换了接口地址。
fn origin(base_url: &str) -> String {
    let (scheme, rest) = base_url.split_once("://").unwrap_or(("", base_url));
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    format!("{scheme}://{host}").to_ascii_lowercase()
}

/// 决定保存时用哪个 key：填了就校验后用新的；没填时沿用旧 key，但换了接口地址（scheme / 主机 / 端口）
/// 必须重新填，否则旧 key 会被发往新地址。`base_url` 为已规范化的地址。
pub(crate) fn resolve_key(typed: &str, old: Option<&CustomApi>, base_url: &str) -> Result<String, String> {
    if !typed.is_empty() {
        validate_key(typed)?;
        return Ok(typed.to_string());
    }
    match old {
        None => Err("请填写 API Key".into()),
        Some(o) if origin(&o.base_url) != origin(base_url) => Err("更换了接口地址，请重新填写 API Key".into()),
        Some(o) => Ok(o.api_key.clone()),
    }
}

/// 重新校验后更新条目：通过则刷新时间与认证方式；判定类失败清除已验证；网络类失败保持原状。
pub(crate) fn apply_reverify(e: &mut CustomApi, report: &VerifyReport, at: u64) {
    if report.ok {
        e.verified_at = Some(at);
        if let Some(a) = report.auth_style.clone() {
            e.auth_style = a;
        }
    } else if !report.transient {
        e.verified_at = None;
    }
}

/// 是否应当接管：Codex 模型和自定义 API 任一非空即可。
pub(crate) fn want_takeover(onboarded: bool, takeover: bool, running: bool, codex_empty: bool, custom_empty: bool) -> bool {
    onboarded && takeover && running && !(codex_empty && custom_empty)
}

pub struct Controller {
    pub app: AppHandle,
    pub sup: Supervisor,
    home: PathBuf,
    data_dir: PathBuf,
    custom: Mutex<Vec<CustomApi>>,
    cfg: Mutex<AppConfig>,
    rec: Mutex<TakeoverRecord>,
    onboarded: AtomicBool,
    settings_error: Mutex<Option<String>>,
    auth: Mutex<AuthState>,
    tx: mpsc::Sender<()>,
    shut: AtomicBool,
    on_status: Mutex<Option<Box<dyn Fn(&StatusDto) + Send>>>,
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

impl Controller {
    pub fn new(app: AppHandle, home: PathBuf, data_dir: PathBuf) -> Arc<Self> {
        let cfg_path = data_dir.join("config.json");
        let onboarded = cfg_path.exists();
        let cfg: AppConfig = load_json(&cfg_path);
        let rec: TakeoverRecord = load_json(&data_dir.join("takeover.json"));
        let custom = crate::custom_apis::load(&data_dir);
        custom_api::set_entries(custom.clone());
        let (tx, rx) = mpsc::channel::<()>();
        let tx2 = tx.clone();
        let sup = Supervisor::new(home.clone(), cfg.clone(), move || {
            let _ = tx2.send(());
        });
        let me = Arc::new(Self {
            app,
            sup,
            home,
            data_dir,
            custom: Mutex::new(custom),
            cfg: Mutex::new(cfg),
            rec: Mutex::new(rec),
            onboarded: AtomicBool::new(onboarded),
            settings_error: Mutex::new(None),
            auth: Mutex::new(AuthState::Missing { code: String::new() }),
            tx,
            shut: AtomicBool::new(false),
            on_status: Mutex::new(None),
        });
        me.refresh_auth();
        // 崩溃恢复：上次没撤销干净的先撤销
        me.revoke_now();
        let worker = me.clone();
        std::thread::spawn(move || worker.event_loop(rx));
        me
    }

    pub fn set_status_listener(&self, f: impl Fn(&StatusDto) + Send + 'static) {
        *self.on_status.lock().unwrap() = Some(Box::new(f));
    }

    pub fn notify(&self) {
        let _ = self.tx.send(());
    }

    pub fn config(&self) -> AppConfig {
        self.cfg.lock().unwrap().clone()
    }
    pub fn onboarded(&self) -> bool {
        self.onboarded.load(Ordering::SeqCst)
    }
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// 开发模式下未指定 Claude 目录时，写沙盒文件，不碰全局 ~/.claude/settings.json。
    pub fn settings_path(&self) -> PathBuf {
        let cfg = self.config();
        if crate::config::DEV_MODE && cfg.claude_dir.is_none() {
            return self.data_dir.join("claude-sandbox").join("settings.json");
        }
        cfg.claude_settings_path(&self.home)
    }

    pub fn auth_path(&self) -> PathBuf {
        let env: HashMap<String, String> = std::env::vars().collect();
        auth_file(self.config().codex_home.as_deref(), &env, &self.home)
    }

    fn event_loop(self: Arc<Self>, rx: mpsc::Receiver<()>) {
        let mut last_mtime = None;
        loop {
            match rx.recv_timeout(Duration::from_secs(5)) {
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
                Ok(()) => while rx.try_recv().is_ok() {},
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if self.shut.load(Ordering::SeqCst) {
                continue;
            }
            let mtime = std::fs::metadata(self.auth_path()).and_then(|m| m.modified()).ok();
            let auth_changed = mtime != last_mtime;
            last_mtime = mtime;
            if auth_changed {
                let was_missing = matches!(*self.auth.lock().unwrap(), AuthState::Missing { .. });
                self.refresh_auth();
                let now_ok = !matches!(*self.auth.lock().unwrap(), AuthState::Missing { .. });
                if was_missing && now_ok && self.onboarded() && self.sup.state() != ProxyState::Stopped {
                    self.sup.restart();
                }
            }
            self.reconcile();
            self.publish();
        }
    }

    fn refresh_auth(&self) {
        let st = inspect_auth(&self.auth_path(), now());
        *self.auth.lock().unwrap() = st;
    }

    fn save_record(&self, rec: &TakeoverRecord) {
        let _ = save_json(&self.data_dir.join("takeover.json"), rec);
    }

    /// 让 settings.json 与期望状态一致。
    pub fn reconcile(&self) {
        if self.shut.load(Ordering::SeqCst) {
            return;
        }
        let cfg = self.config();
        let state = self.sup.state();
        let available = self.sup.models();
        let custom_names = custom_api::picker_list();
        let want = want_takeover(
            self.onboarded(),
            cfg.takeover,
            state == ProxyState::Running,
            available.is_empty(),
            custom_names.is_empty(),
        );
        let models = takeover_models(cfg.effective_models(&available), custom_names);
        let failed = matches!(state, ProxyState::Failed { .. });
        if want {
            self.apply_now(&models);
        } else if failed || !cfg.takeover || !self.onboarded() {
            if self.revoke_now() && failed {
                let _ = self
                    .app
                    .notification()
                    .builder()
                    .title(crate::i18n::tr(cfg.lang(), "notifyFailedTitle"))
                    .body(crate::i18n::tr(cfg.lang(), "notifyFailedBody"))
                    .show();
            }
        }
    }

    fn apply_now(&self, models: &[String]) {
        let path = self.settings_path();
        let mut doc = match read_settings(&path) {
            Ok(d) => d,
            Err(e) => return self.set_settings_error(Some(e)),
        };
        let before = doc.clone();
        let mut rec = self.rec.lock().unwrap();
        let mut next = rec.clone();
        apply_takeover(&mut doc, &mut next, &self.sup.base_url(), models);
        if doc != before {
            // 先落记录再改文件：即使中途崩溃，下次也能识别并撤销
            self.save_record(&next);
            if let Err(e) = write_settings(&path, &doc) {
                return self.set_settings_error(Some(SettingsError::Io(e)));
            }
        } else if next != *rec {
            self.save_record(&next);
        }
        *rec = next;
        self.set_settings_error(None);
    }

    /// 返回是否真的撤销了内容。
    pub fn revoke_now(&self) -> bool {
        let mut rec = self.rec.lock().unwrap();
        if rec.base_url.is_none() && rec.models.is_empty() {
            return false;
        }
        let path = self.settings_path();
        let mut doc = match read_settings(&path) {
            Ok(d) => d,
            Err(e) => {
                self.set_settings_error(Some(e));
                return false;
            }
        };
        if !has_leftover(&doc, &rec) {
            *rec = TakeoverRecord::default();
            self.save_record(&rec);
            return false;
        }
        let before = doc.clone();
        let warnings = revoke_takeover(&mut doc, &mut rec);
        if doc != before {
            if let Err(e) = write_settings(&path, &doc) {
                self.set_settings_error(Some(SettingsError::Io(e)));
                return false;
            }
        }
        self.save_record(&rec);
        for w in warnings {
            self.sup.note(format!("[cc-bridge] {w}"));
        }
        doc != before
    }

    fn set_settings_error(&self, e: Option<SettingsError>) {
        *self.settings_error.lock().unwrap() = e.map(|e| e.to_string());
    }

    pub fn status(&self) -> StatusDto {
        let cfg = self.config();
        let path = self.settings_path();
        let taken_over = read_settings(&path).map(|d| is_taken_over(&d, &self.rec.lock().unwrap())).unwrap_or(false);
        let (auth_hint, network_hint) = match self.sup.last_hint() {
            LogHint::Auth401(m) => (Some(m), None),
            LogHint::UpstreamNetwork(m) => (None, Some(m)),
            LogHint::None => (None, None),
        };
        StatusDto {
            proxy: self.sup.state(),
            port: cfg.port,
            auth: self.auth.lock().unwrap().clone(),
            auth_path: self.auth_path().to_string_lossy().into_owned(),
            auth_hint,
            network_hint,
            takeover: cfg.takeover,
            taken_over,
            settings_path: path.to_string_lossy().into_owned(),
            settings_error: self.settings_error.lock().unwrap().clone(),
            selected_models: cfg.effective_models(&self.sup.models()),
            models: self.sup.models(),
            lang: cfg.lang().to_string(),
            config: cfg,
            logs_tail: self.sup.logs_tail(200),
            onboarded: self.onboarded(),
            dev_mode: crate::config::DEV_MODE,
            custom_apis: crate::custom_apis::to_dtos(&self.custom.lock().unwrap()),
        }
    }

    /// 写盘、更新代理路由表并触发一次 reconcile。
    fn commit_custom(&self, v: Vec<CustomApi>) -> Result<(), String> {
        crate::custom_apis::save(&self.data_dir, &v).map_err(|e| format!("保存自定义 API 失败：{e}"))?;
        custom_api::set_entries(v.clone());
        *self.custom.lock().unwrap() = v;
        self.notify();
        Ok(())
    }

    async fn run_verify(&self, c: &CustomApi) -> VerifyReport {
        let client = custom_api::provider::build_client();
        verify(&client, &c.base_url, &c.api_key, &c.model, |s, r| {
            let _ = self.app.emit("custom-api-step", serde_json::json!({ "id": c.id, "step": s, "result": r }));
        })
        .await
    }

    pub async fn save_custom_api(&self, input: CustomApiInput) -> Result<VerifyReport, String> {
        let base_url = custom_api::normalize_base_url(&input.base_url)?;
        let model = input.model.trim().to_string();
        custom_api::validate_model(&model)?;
        let existing = self.custom.lock().unwrap().clone();
        let old = input.id.as_ref().and_then(|id| existing.iter().find(|e| &e.id == id)).cloned();
        let typed = input.api_key.as_deref().map(str::trim).unwrap_or("");
        let api_key = resolve_key(typed, old.as_ref(), &base_url)?;
        let mut cand = CustomApi {
            id: input.id.clone().unwrap_or_else(new_id),
            base_url,
            api_key,
            model,
            auth_style: old.as_ref().map(|o| o.auth_style.clone()).unwrap_or(AuthStyle::XApiKey),
            verified_at: None,
        };
        let report = self.run_verify(&cand).await;
        if report.ok {
            cand.verified_at = Some(now() as u64);
            if let Some(a) = report.auth_style.clone() {
                cand.auth_style = a;
            }
        }
        // 校验期间可能有其它增删，这里重新取当前列表再合并
        let current = self.custom.lock().unwrap().clone();
        self.commit_custom(merge_edit(&current, cand, report.ok))?;
        Ok(report)
    }

    pub async fn reverify_custom_api(&self, id: String) -> Result<VerifyReport, String> {
        let entry = self.custom.lock().unwrap().iter().find(|e| e.id == id).cloned().ok_or_else(|| "该条目已不存在".to_string())?;
        let report = self.run_verify(&entry).await;
        let mut v = self.custom.lock().unwrap().clone();
        if let Some(e) = v.iter_mut().find(|e| e.id == id) {
            apply_reverify(e, &report, now() as u64);
        }
        self.commit_custom(v)?;
        Ok(report)
    }

    pub fn delete_custom_api(&self, id: &str) -> Result<(), String> {
        let mut v = self.custom.lock().unwrap().clone();
        v.retain(|e| e.id != id);
        self.commit_custom(v)
    }

    pub fn publish(&self) {
        let st = self.status();
        let _ = self.app.emit("status-changed", &st);
        if let Some(f) = self.on_status.lock().unwrap().as_ref() {
            f(&st);
        }
    }

    pub fn set_models(&self, models: Option<Vec<String>>) {
        let mut cfg = self.config();
        cfg.selected_models = models;
        self.store_config(cfg);
        self.notify();
    }

    pub fn set_takeover(&self, on: bool) {
        let mut cfg = self.config();
        cfg.takeover = on;
        self.store_config(cfg);
        self.notify();
    }

    fn store_config(&self, cfg: AppConfig) {
        let _ = save_json(&self.data_dir.join("config.json"), &cfg);
        *self.cfg.lock().unwrap() = cfg.clone();
        self.sup.set_config(cfg);
    }

    /// 保存设置；影响代理或 settings.json 位置的改动会先撤销再重启代理。
    pub fn save_config(&self, cfg: AppConfig) {
        let old = self.config();
        let restart = old.port != cfg.port || old.outbound() != cfg.outbound() || old.codex_home != cfg.codex_home;
        if restart || old.claude_dir != cfg.claude_dir {
            self.revoke_now();
        }
        self.store_config(cfg);
        self.onboarded.store(true, Ordering::SeqCst);
        if restart {
            self.sup.restart();
        }
        self.refresh_auth();
        self.notify();
    }

    /// 引导第 4 步：按引导里的配置先启动代理（尚不保存、不接管）。
    pub fn preview_start(&self, cfg: AppConfig) {
        *self.cfg.lock().unwrap() = cfg.clone();
        self.sup.set_config(cfg);
        self.refresh_auth();
        self.sup.restart();
    }

    pub fn finish_onboarding(&self, cfg: AppConfig) {
        self.store_config(cfg);
        self.onboarded.store(true, Ordering::SeqCst);
        if self.sup.state() == ProxyState::Stopped {
            self.sup.start();
        }
        self.notify();
    }

    /// 退出 / 关机：撤销接管并结束代理。可重复调用。
    pub fn shutdown(&self) {
        if self.shut.swap(true, Ordering::SeqCst) {
            return;
        }
        self.revoke_now();
        self.sup.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::custom_api::AuthStyle;

    fn api(id: &str, verified: Option<u64>) -> CustomApi {
        CustomApi {
            id: id.into(),
            base_url: "https://relay.example.com".into(),
            api_key: "k".into(),
            model: "claude-opus-5-5".into(),
            auth_style: AuthStyle::XApiKey,
            verified_at: verified,
        }
    }

    #[test]
    fn takeover_with_only_custom_apis() {
        let v = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(takeover_models(vec![], v(&["api-a"])), v(&["api-a"]));
        assert_eq!(takeover_models(v(&["gpt-a"]), v(&["api-a"])), v(&["gpt-a", "api-a"]));
        assert!(want_takeover(true, true, true, true, false));
        assert!(!want_takeover(true, true, true, true, true));
        assert!(want_takeover(true, true, true, false, true));
        assert!(!want_takeover(false, true, true, false, false));
        assert!(!want_takeover(true, false, true, false, false));
        assert!(!want_takeover(true, true, false, false, false));
    }

    #[test]
    fn empty_key_with_changed_host_is_rejected() {
        let old = api("a", Some(1));
        assert_eq!(resolve_key("", Some(&old), "https://relay.example.com/anthropic").unwrap(), "k");
        assert_eq!(resolve_key("", Some(&old), "https://RELAY.example.com").unwrap(), "k");
        for moved in ["https://evil.example.com", "https://relay.example.com:8443", "http://relay.example.com"] {
            assert_eq!(
                resolve_key("", Some(&old), moved).unwrap_err(),
                "更换了接口地址，请重新填写 API Key",
                "{moved}"
            );
        }
        assert_eq!(resolve_key("sk-new", Some(&old), "https://evil.example.com").unwrap(), "sk-new");
        assert_eq!(resolve_key("", None, "https://relay.example.com").unwrap_err(), "请填写 API Key");
        assert!(resolve_key("sk bad", None, "https://relay.example.com").is_err());
    }

    fn report(ok: bool, transient: bool) -> VerifyReport {
        VerifyReport { steps: vec![], auth_style: Some(AuthStyle::Bearer), ok, transient }
    }

    #[test]
    fn reverify_keeps_verified_on_network_failure() {
        let mut e = api("a", Some(1));
        apply_reverify(&mut e, &report(false, true), 9);
        assert_eq!(e.verified_at, Some(1));
        assert_eq!(e.auth_style, AuthStyle::XApiKey);
        apply_reverify(&mut e, &report(true, false), 9);
        assert_eq!((e.verified_at, e.auth_style.clone()), (Some(9), AuthStyle::Bearer));
        apply_reverify(&mut e, &report(false, false), 10);
        assert_eq!(e.verified_at, None);
    }

    #[test]
    fn merge_edit_via_controller_module() {
        let existing = vec![api("a", Some(1))];
        assert_eq!(crate::custom_apis::merge_edit(&existing, api("a", None), false), existing);
    }
}
