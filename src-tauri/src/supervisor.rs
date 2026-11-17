//! 代理进程监护：拉起、健康检查、退避重启、日志收集、端口冲突识别。
use crate::{config::AppConfig, env_builder::build_proxy_env, log_buffer::LogBuffer, proc_guard};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{HashMap, VecDeque},
    io::{BufRead, BufReader, Read},
    net::TcpListener,
    path::PathBuf,
    process::{Child, Command, Stdio},
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
#[serde(tag = "kind", rename_all = "camelCase")]
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

/// 解析上游 `CCP_LOG_STDERR` 的 JSON 行。成功请求返回 `Some(LogHint::None)`（用于清除旧提示），无关行返回 `None`。
pub fn classify_log_line(line: &str) -> Option<LogHint> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    let msg = v.get("msg")?.as_str()?;
    let f = v.get("fields").cloned().unwrap_or(Value::Null);
    let status = f.get("status").and_then(Value::as_u64);
    let text = |k: &str| f.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    match msg {
        "request_failed" if status == Some(401) => Some(LogHint::Auth401(text("message"))),
        "codex_upstream_request_failed" if f.get("origin").and_then(Value::as_str) != Some("Auth") && status != Some(401) => {
            let err = text("error");
            let lower = err.to_lowercase();
            let net = ["network", "timed out", "timeout", "connect", "proxy", "dns", "tls"].iter().any(|k| lower.contains(k));
            net.then_some(LogHint::UpstreamNetwork(err))
        }
        "request_completed" if status == Some(200) && f.get("provider").and_then(Value::as_str) == Some("codex") => Some(LogHint::None),
        _ => None,
    }
}

type Listener = Box<dyn Fn() + Send + Sync>;

struct Inner {
    sidecar: PathBuf,
    home: PathBuf,
    cfg: Mutex<AppConfig>,
    state: Mutex<ProxyState>,
    child: Mutex<Option<Child>>,
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
    pub fn new(sidecar: PathBuf, home: PathBuf, cfg: AppConfig, on_change: impl Fn() + Send + Sync + 'static) -> Self {
        Self(Arc::new(Inner {
            sidecar,
            home,
            cfg: Mutex::new(cfg),
            state: Mutex::new(ProxyState::Stopped),
            child: Mutex::new(None),
            models: Mutex::new(vec![]),
            hint: Mutex::new(LogHint::None),
            logs: LogBuffer::default(),
            restarts: Mutex::new(VecDeque::new()),
            generation: AtomicU64::new(0),
            on_change: Box::new(on_change),
        }))
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

    /// 同步结束代理进程。
    pub fn stop(&self) {
        self.0.generation.fetch_add(1, Ordering::SeqCst);
        self.kill_child();
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

    fn kill_child(&self) {
        if let Some(mut c) = self.0.child.lock().unwrap().take() {
            let _ = c.kill();
            let _ = c.wait();
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
                self.log(format!("[codex-bridge] 端口 {port} 已被占用 pid={pid:?}"));
                self.set_state(ProxyState::PortBusy { pid, is_claude_codex: is_cc });
                return;
            }
            self.set_state(ProxyState::Starting);
            match self.spawn(port) {
                Ok(()) => self.watch(generation, &client),
                Err(e) => self.log(format!("[codex-bridge] 启动代理失败：{e}")),
            }
            self.kill_child();
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
                    self.set_state(ProxyState::Failed { message: format!("代理在 1 分钟内反复退出，已停止自动重启。\n{tail}") });
                    return;
                }
            }
        }
    }

    fn spawn(&self, port: u16) -> std::io::Result<()> {
        let cfg = self.0.cfg.lock().unwrap().clone();
        let base: HashMap<String, String> = std::env::vars().collect();
        let mut cmd = Command::new(&self.0.sidecar);
        cmd.args(["serve", "--no-monitor", "--port", &port.to_string()])
            .env_clear()
            .envs(build_proxy_env(&base, &self.0.home, &cfg))
            .current_dir(&self.0.home)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        proc_guard::prepare(&mut cmd);
        let mut child = cmd.spawn()?;
        proc_guard::attach_kill_on_close(&child);
        self.log(format!("[codex-bridge] 代理已启动 pid={} 端口 {port}", child.id()));
        if let Some(out) = child.stdout.take() {
            self.pipe(out);
        }
        if let Some(err) = child.stderr.take() {
            self.pipe(err);
        }
        *self.0.child.lock().unwrap() = Some(child);
        Ok(())
    }

    fn pipe(&self, r: impl Read + Send + 'static) {
        let me = self.clone();
        thread::spawn(move || {
            for line in BufReader::new(r).lines().map_while(Result::ok) {
                if !line.trim().is_empty() {
                    me.log(line);
                }
            }
        });
    }

    fn exited(&self) -> bool {
        match self.0.child.lock().unwrap().as_mut() {
            Some(c) => !matches!(c.try_wait(), Ok(None)),
            None => true,
        }
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
                self.log("[codex-bridge] 代理启动超时".into());
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
                self.log("[codex-bridge] 代理进程已退出".into());
                return;
            }
            if self.probe(client, &url) {
                fails = 0;
            } else {
                fails += 1;
                if fails >= MAX_HEALTH_FAILS {
                    self.log("[codex-bridge] 健康检查连续失败，重启代理".into());
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
        let ids: Vec<String> = v
            .get("data")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|m| m.get("id").and_then(Value::as_str)).filter(|id| !id.starts_with("claude")).map(String::from).collect())
            .unwrap_or_default();
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
    }
}
