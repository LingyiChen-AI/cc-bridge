//! 代理监护：在本进程内启动代理服务、健康检查、退避重启、日志收集、端口冲突识别。
use crate::{config::AppConfig, log_buffer::LogBuffer};
use crate::proxy::overrides::{self, RuntimeOverrides};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::VecDeque,
    net::TcpListener,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const HEALTH_INTERVAL: Duration = Duration::from_secs(10);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(20);
const RESTART_WINDOW: Duration = Duration::from_secs(60);
const MAX_HEALTH_FAILS: u32 = 3;

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ProxyState {
    Starting,
    Running,
    Restarting { attempt: u32 },
    Failed { message: String },
    PortBusy { pid: Option<u32>, is_claude_codex: bool },
    Stopped,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", content = "message", rename_all = "camelCase")]
pub enum LogHint {
    None,
    Auth401(String),
    UpstreamNetwork(String),
}

pub fn next_delay(restarts_in_window: usize) -> Option<Duration> {
    [1, 2, 5, 10, 30].get(restarts_in_window).map(|s| Duration::from_secs(*s))
}

/// 解析代理结构化日志的 JSON 行。成功请求返回 `Some(LogHint::None)`（用于清除旧提示），无关行返回 `None`。
pub fn classify_log_line(line: &str) -> Option<LogHint> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    let msg = v.get("msg")?.as_str()?;
    let f = v.get("fields").cloned().unwrap_or(Value::Null);
    let status = f.get("status").and_then(Value::as_u64);
    let text = |k: &str| f.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    match msg {
        "request_failed" if status == Some(401) => Some(LogHint::Auth401(text("message"))),
        // 上游连不上时，失败信息出现在 request_failed.message（502）或 codex_upstream_request_failed.error
        "request_failed" | "codex_upstream_request_failed" if f.get("origin").and_then(Value::as_str) != Some("Auth") => {
            let err = if msg == "request_failed" { text("message") } else { text("error") };
            is_network_error(&err).then_some(LogHint::UpstreamNetwork(err))
        }
        "request_completed" if status == Some(200) && f.get("provider").and_then(Value::as_str) == Some("codex") => Some(LogHint::None),
        _ => None,
    }
}

fn is_network_error(err: &str) -> bool {
    let lower = err.to_lowercase();
    ["network", "timed out", "timeout", "connect", "proxy", "dns", "tls", "error sending request"].iter().any(|k| lower.contains(k))
}

/// 只保留 Codex 提供的模型（display_name 以 "(codex)" 结尾），新版本在前。
pub fn codex_models(data: &[Value]) -> Vec<String> {
    let mut ids: Vec<String> = data
        .iter()
        .filter(|m| m.get("display_name").and_then(Value::as_str).is_some_and(|d| d.trim_end().ends_with("(codex)")))
        .filter_map(|m| m.get("id").and_then(Value::as_str).map(String::from))
        .collect();
    ids.sort_by(|a, b| version_key(b).cmp(&version_key(a)).then_with(|| a.cmp(b)));
    ids
}

/// "gpt-6.1-sol-fast" → [6, 1]；没有版本号的排最后。
fn version_key(id: &str) -> Vec<u32> {
    id.split('-')
        .find(|seg| seg.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(|v| v.split('.').map(|n| n.parse().unwrap_or(0)).collect())
        .unwrap_or_default()
}

/// 由客户端配置得到代理的进程内覆盖：HOME、登录文件、出口代理全部显式传入，不依赖环境变量。
pub fn proxy_overrides(cfg: &AppConfig, home: &Path) -> RuntimeOverrides {
    RuntimeOverrides {
        home: Some(home.to_path_buf()),
        codex_auth_file: cfg.codex_home.as_ref().map(|d| d.join("auth.json")),
        outbound_proxy: cfg.outbound().map(String::from),
        no_proxy: Some("localhost,127.0.0.1,::1".into()),
        direct: cfg.outbound().is_none(),
    }
}

/// 一次运行中的代理服务。
struct Running {
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<anyhow::Result<()>>,
}

type Listener = Box<dyn Fn() + Send + Sync>;

struct Inner {
    home: PathBuf,
    rt: tokio::runtime::Runtime,
    cfg: Mutex<AppConfig>,
    state: Mutex<ProxyState>,
    server: Mutex<Option<Running>>,
    models: Mutex<Vec<String>>,
    hint: Mutex<LogHint>,
    logs: LogBuffer,
    restarts: Mutex<VecDeque<Instant>>,
    generation: AtomicU64,
    on_change: Listener,
}

#[derive(Clone)]
pub struct Supervisor(Arc<Inner>);

impl Supervisor {
    pub fn new(home: PathBuf, cfg: AppConfig, on_change: impl Fn() + Send + Sync + 'static) -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .thread_name("codex-proxy")
            .enable_all()
            .build()
            .expect("tokio runtime");
        let me = Self(Arc::new(Inner {
            home,
            rt,
            cfg: Mutex::new(cfg),
            state: Mutex::new(ProxyState::Stopped),
            server: Mutex::new(None),
            models: Mutex::new(vec![]),
            hint: Mutex::new(LogHint::None),
            logs: LogBuffer::default(),
            restarts: Mutex::new(VecDeque::new()),
            generation: AtomicU64::new(0),
            on_change: Box::new(on_change),
        }));
        // 代理的每条结构化日志都进入界面日志，并参与 401 / 网络错误识别
        let sink = me.clone();
        overrides::set_log_sink(move |line| sink.log(line.to_string()));
        me
    }

    pub fn state(&self) -> ProxyState {
        self.0.state.lock().unwrap().clone()
    }
    pub fn models(&self) -> Vec<String> {
        self.0.models.lock().unwrap().clone()
    }
    pub fn logs(&self) -> Vec<String> {
        self.0.logs.all()
    }
    pub fn logs_tail(&self, n: usize) -> Vec<String> {
        self.0.logs.tail(n)
    }
    pub fn last_hint(&self) -> LogHint {
        self.0.hint.lock().unwrap().clone()
    }
    pub fn port(&self) -> u16 {
        self.0.cfg.lock().unwrap().port
    }
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port())
    }
    pub fn set_config(&self, cfg: AppConfig) {
        *self.0.cfg.lock().unwrap() = cfg;
    }

    pub fn note(&self, line: String) {
        self.log(line);
    }

    pub fn start(&self) {
        let generation = self.0.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let me = self.clone();
        thread::spawn(move || me.run(generation));
    }

    pub fn restart(&self) {
        self.stop();
        self.0.restarts.lock().unwrap().clear();
        self.start();
    }

    /// 同步停止代理服务。
    pub fn stop(&self) {
        self.0.generation.fetch_add(1, Ordering::SeqCst);
        self.stop_server();
        self.set_state(ProxyState::Stopped);
    }

    pub fn kill_port_owner_and_start(&self) {
        if let Some(pid) = port_owner(self.port()) {
            let sys = sysinfo::System::new_with_specifics(sysinfo::RefreshKind::nothing().with_processes(sysinfo::ProcessRefreshKind::nothing()));
            if let Some(p) = sys.process(sysinfo::Pid::from_u32(pid)) {
                p.kill();
            }
            thread::sleep(Duration::from_millis(800));
        }
        self.restart();
    }

    fn alive(&self, generation: u64) -> bool {
        self.0.generation.load(Ordering::SeqCst) == generation
    }

    fn set_state(&self, s: ProxyState) {
        let changed = {
            let mut cur = self.0.state.lock().unwrap();
            let changed = *cur != s;
            *cur = s;
            changed
        };
        if changed {
            (self.0.on_change)();
        }
    }

    fn log(&self, line: String) {
        if let Some(h) = classify_log_line(&line) {
            let changed = {
                let mut cur = self.0.hint.lock().unwrap();
                let changed = *cur != h;
                *cur = h;
                changed
            };
            if changed {
                (self.0.on_change)();
            }
        }
        self.0.logs.push(line);
    }

    fn stop_server(&self) {
        let Some(mut running) = self.0.server.lock().unwrap().take() else { return };
        if let Some(tx) = running.shutdown.take() {
            let _ = tx.send(());
        }
        // 优雅关闭最多等 3 秒，超时直接中止（长连接流式请求可能一直不结束）
        let task = running.task;
        let finished = self.0.rt.block_on(async {
            let abort = task.abort_handle();
            match tokio::time::timeout(Duration::from_secs(3), task).await {
                Ok(_) => true,
                Err(_) => {
                    abort.abort();
                    false
                }
            }
        });
        if !finished {
            self.log("[cc-bridge] proxy did not stop within 3s, aborted".into());
        }
    }

    fn sleep_while_alive(&self, generation: u64, d: Duration) -> bool {
        let end = Instant::now() + d;
        while Instant::now() < end {
            if !self.alive(generation) {
                return false;
            }
            thread::sleep(Duration::from_millis(200));
        }
        self.alive(generation)
    }

    fn run(&self, generation: u64) {
        let client = http_client();
        loop {
            if !self.alive(generation) {
                return;
            }
            let port = self.port();
            if TcpListener::bind(("127.0.0.1", port)).is_err() {
                let pid = port_owner(port);
                let is_cc = pid.and_then(process_name).is_some_and(|n| n.to_lowercase().starts_with("claude-codex"));
                self.log(format!("[cc-bridge] port {port} is in use, pid={pid:?}"));
                self.set_state(ProxyState::PortBusy { pid, is_claude_codex: is_cc });
                return;
            }
            self.set_state(ProxyState::Starting);
            self.spawn(port);
            self.watch(generation, &client);
            self.stop_server();
            if !self.alive(generation) {
                return;
            }
            // 需要重启
            let count = {
                let mut r = self.0.restarts.lock().unwrap();
                let now = Instant::now();
                while r.front().is_some_and(|t| now.duration_since(*t) > RESTART_WINDOW) {
                    r.pop_front();
                }
                r.push_back(now);
                r.len() - 1
            };
            match next_delay(count) {
                Some(d) => {
                    self.set_state(ProxyState::Restarting { attempt: count as u32 + 1 });
                    if !self.sleep_while_alive(generation, d) {
                        return;
                    }
                }
                None => {
                    let tail = self.0.logs.tail(5).join("\n");
                    self.set_state(ProxyState::Failed { message: tail });
                    return;
                }
            }
        }
    }

    fn spawn(&self, port: u16) {
        let cfg = self.0.cfg.lock().unwrap().clone();
        overrides::set(proxy_overrides(&cfg, &self.0.home));
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let config = crate::proxy::server::ServerConfig { bind_address: "127.0.0.1".into(), port, monitor: None };
        let task = self.0.rt.spawn(crate::proxy::server::serve_with_shutdown(config, async {
            let _ = rx.await;
        }));
        self.log(format!(
            "[cc-bridge] proxy started in-process on port {port}, outbound {}",
            cfg.outbound().unwrap_or("direct")
        ));
        *self.0.server.lock().unwrap() = Some(Running { shutdown: Some(tx), task });
    }

    /// 代理服务任务已结束（启动失败、panic 或被关闭）。
    fn exited(&self) -> bool {
        let mut guard = self.0.server.lock().unwrap();
        let Some(running) = guard.as_mut() else { return true };
        if !running.task.is_finished() {
            return false;
        }
        let task = guard.take().unwrap().task;
        let msg = match self.0.rt.block_on(task) {
            Ok(Ok(())) => "proxy server exited".to_string(),
            Ok(Err(e)) => format!("proxy server failed: {e:#}"),
            Err(e) if e.is_panic() => "proxy server panicked".to_string(),
            Err(e) => format!("proxy server aborted: {e}"),
        };
        drop(guard);
        self.log(format!("[cc-bridge] {msg}"));
        true
    }

    /// 返回即表示需要重启（或已被 stop）。
    fn watch(&self, generation: u64, client: &reqwest::blocking::Client) {
        let url = format!("{}/v1/models", self.base_url());
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        loop {
            if !self.alive(generation) || self.exited() {
                return;
            }
            if self.probe(client, &url) {
                break;
            }
            if Instant::now() > deadline {
                self.log("[cc-bridge] proxy startup timed out".into());
                return;
            }
            thread::sleep(Duration::from_millis(400));
        }
        self.set_state(ProxyState::Running);
        let mut fails = 0;
        loop {
            if !self.sleep_while_alive(generation, HEALTH_INTERVAL) {
                return;
            }
            if self.exited() {
                return;
            }
            if self.probe(client, &url) {
                fails = 0;
            } else {
                fails += 1;
                if fails >= MAX_HEALTH_FAILS {
                    self.log("[cc-bridge] health check failed repeatedly, restarting proxy".into());
                    return;
                }
            }
        }
    }

    pub fn refresh_models(&self) -> bool {
        self.probe(&http_client(), &format!("{}/v1/models", self.base_url()))
    }

    fn probe(&self, client: &reqwest::blocking::Client, url: &str) -> bool {
        let Ok(resp) = client.get(url).send() else { return false };
        if !resp.status().is_success() {
            return false;
        }
        let Ok(v) = resp.json::<Value>() else { return false };
        let ids = v.get("data").and_then(Value::as_array).map(|a| codex_models(a)).unwrap_or_default();
        let changed = {
            let mut cur = self.0.models.lock().unwrap();
            let changed = *cur != ids;
            *cur = ids;
            changed
        };
        if changed {
            (self.0.on_change)();
        }
        true
    }
}

fn http_client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder().no_proxy().timeout(Duration::from_secs(5)).build().expect("http client")
}

pub fn port_owner(port: u16) -> Option<u32> {
    use netstat2::*;
    let sockets = get_sockets_info(AddressFamilyFlags::IPV4 | AddressFamilyFlags::IPV6, ProtocolFlags::TCP).ok()?;
    sockets.into_iter().find_map(|s| match &s.protocol_socket_info {
        ProtocolSocketInfo::Tcp(t) if t.local_port == port && t.state == TcpState::Listen => s.associated_pids.first().copied(),
        _ => None,
    })
}

fn process_name(pid: u32) -> Option<String> {
    let sys = sysinfo::System::new_with_specifics(sysinfo::RefreshKind::nothing().with_processes(sysinfo::ProcessRefreshKind::nothing()));
    sys.process(sysinfo::Pid::from_u32(pid)).map(|p| p.name().to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_from_config() {
        let home = Path::new(r"C:\Users\张 三");
        let mut cfg = AppConfig { outbound_proxy: Some("http://127.0.0.1:7890".into()), ..Default::default() };
        let o = proxy_overrides(&cfg, home);
        assert_eq!(o.home.as_deref(), Some(home));
        assert_eq!(o.outbound_proxy.as_deref(), Some("http://127.0.0.1:7890"));
        assert!(!o.direct);
        assert_eq!(o.codex_auth_file, None);
        cfg.outbound_proxy = Some("  ".into());
        cfg.codex_home = Some(PathBuf::from("D:/codex"));
        let o = proxy_overrides(&cfg, home);
        assert!(o.direct && o.outbound_proxy.is_none());
        assert_eq!(o.codex_auth_file, Some(PathBuf::from("D:/codex").join("auth.json")));
    }

    #[test]
    fn backoff_sequence() {
        let secs: Vec<u64> = (0..5).map(|i| next_delay(i).unwrap().as_secs()).collect();
        assert_eq!(secs, [1, 2, 5, 10, 30]);
        assert_eq!(next_delay(5), None);
    }

    #[test]
    fn classify_401() {
        let l = r#"{"msg":"request_failed","fields":{"status":401,"message":"No Codex credentials. x","provider":"codex"}}"#;
        assert_eq!(classify_log_line(l), Some(LogHint::Auth401("No Codex credentials. x".into())));
    }

    #[test]
    fn classify_network() {
        let l = r#"{"level":"warn","msg":"codex_upstream_request_failed","fields":{"error":"Network error: operation timed out","status":502,"origin":"Transport"}}"#;
        assert!(matches!(classify_log_line(l), Some(LogHint::UpstreamNetwork(_))));
        let ok = r#"{"level":"info","msg":"request","fields":{"method":"POST"}}"#;
        assert_eq!(classify_log_line(ok), None);
        let done = r#"{"msg":"request_completed","fields":{"status":200,"provider":"codex"}}"#;
        assert_eq!(classify_log_line(done), Some(LogHint::None));
        assert_eq!(classify_log_line("Configure Claude Code:"), None);
        let failed = r#"{"msg":"request_failed","fields":{"status":502,"message":"Network error: error sending request for url (https://chatgpt.com/backend-api/codex/responses)"}}"#;
        assert!(matches!(classify_log_line(failed), Some(LogHint::UpstreamNetwork(_))));
        let rate = r#"{"msg":"request_failed","fields":{"status":429,"message":"Error","model":"claude-opus-5-5"}}"#;
        assert_eq!(classify_log_line(rate), None);
    }

    #[test]
    fn codex_models_filtered_and_newest_first() {
        let data: Vec<Value> = [("claude-opus-5-5", "anthropic"), ("gpt-5.2", "codex"), ("gpt-6.1-sol-fast", "codex"), ("gpt-6-sol", "codex"),
            ("gpt-6.1-sol", "codex"), ("grok-4.7", "grok"), ("cursor", "cursor"), ("gpt-5.3-codex", "codex")]
            .iter()
            .map(|(id, p)| serde_json::json!({"id": id, "display_name": format!("{id} ({p})")}))
            .collect();
        assert_eq!(codex_models(&data), ["gpt-6.1-sol", "gpt-6.1-sol-fast", "gpt-6-sol", "gpt-5.3-codex", "gpt-5.2"]);
    }
}
