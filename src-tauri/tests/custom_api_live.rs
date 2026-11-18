//! 自定义 Claude API 真实接口实测（需要网络与真实 key，默认忽略）。
//!
//! 运行（key 只经环境变量传入，测试不打印 key）：
//!   CUSTOM_API_BASE=https://api.modelverse.cn CUSTOM_API_KEY_VAR=UCLOUD_KEY \
//!   DEEPSEEK_API_KEY=... UCLOUD_KEY=... \
//!   cargo test --test custom_api_live -- --ignored --nocapture --test-threads=1
//!
//! - `relay_*`：真 Claude 中转 → `verify()` 全过；起代理把 `api-<model>` 转给中转，
//!   用 claude.exe `-p` 走一遍；最后在日志、抓包、CLAUDE_CONFIG_DIR、claude 输出里查 key 后 8 位应为 0 命中。
//! - `deepseek_*`：DeepSeek 的 Anthropic 兼容接口冒充 `claude-haiku-5-5` → `verify()` 不通过。
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use cc_bridge_lib::proxy::custom_api::verify::{StepResult, VerifyReport, verify};
use cc_bridge_lib::proxy::custom_api::{AuthStyle, CustomApi, set_entries};
use cc_bridge_lib::proxy::overrides;
use cc_bridge_lib::proxy::server::{ServerConfig, serve_with_shutdown};

const MODEL: &str = "claude-haiku-5-5";
const CLAUDE_EXE: &str = r"C:\bench-data\bin\claude.exe";

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| default.to_string())
}

fn secret(var: &str) -> String {
    std::env::var(var)
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| panic!("需要环境变量 {var}"))
}

/// 隔离的状态目录（日志、抓包），整个测试进程共用一份；首次调用时设好环境变量。
fn sandbox() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("ccb-custom-api-live-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("state")).unwrap();
        std::fs::create_dir_all(dir.join("config")).unwrap();
        std::fs::create_dir_all(dir.join("claude-config")).unwrap();
        // 单线程测试进程内设置，代理与 claude 子进程都从这里读。
        unsafe {
            std::env::set_var("LOCALAPPDATA", dir.join("state"));
            std::env::set_var("XDG_STATE_HOME", dir.join("state"));
            std::env::set_var("CCP_CONFIG_DIR", dir.join("config"));
            std::env::set_var("CCP_TRAFFIC_LOG", "1");
            std::env::remove_var("CCP_AUTH_TOKEN");
        }
        dir
    })
}

fn http_client() -> reqwest::Client {
    // 默认读 HTTPS_PROXY 等环境变量（本机经 127.0.0.1:7890 出网）。
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

fn describe(report: &VerifyReport) -> String {
    report
        .steps
        .iter()
        .map(|(id, r)| match r {
            StepResult::Pass => format!("{id:?}=Pass"),
            StepResult::Skip { reason } => format!("{id:?}=Skip({reason})"),
            StepResult::Fail { reason } => format!("{id:?}=Fail({reason})"),
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

fn count_hits(dir: &Path, needle: &str) -> (usize, usize) {
    let (mut files, mut hits) = (0, 0);
    let Ok(rd) = std::fs::read_dir(dir) else {
        return (0, 0);
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let (f, h) = count_hits(&path, needle);
            files += f;
            hits += h;
        } else if let Ok(bytes) = std::fs::read(&path) {
            files += 1;
            hits += String::from_utf8_lossy(&bytes).matches(needle).count();
        }
    }
    (files, hits)
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

#[tokio::test]
#[ignore = "需要真实中转 key（UCLOUD_KEY）与网络"]
async fn relay_verifies_and_serves_claude_code() {
    let dir = sandbox().clone();
    let base = env_or("CUSTOM_API_BASE", "https://api.modelverse.cn/v1/messages");
    let key = secret(&env_or("CUSTOM_API_KEY_VAR", "UCLOUD_KEY"));
    let tail: String = key.chars().rev().take(8).collect::<Vec<_>>().into_iter().rev().collect();

    let logs: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = logs.clone();
    overrides::set_log_sink(move |line| sink.lock().unwrap().push(line.to_string()));

    // (a) 校验
    let report = verify(&http_client(), &base, &key, MODEL, |_, _| {}).await;
    let verify_desc = describe(&report);
    println!("[relay verify] ok={} auth={:?} {verify_desc}", report.ok, report.auth_style);

    // (c) 端到端：路由表放一个已验证条目（不依赖 (a) 是否通过，便于同时观察两部分）
    set_entries(vec![CustomApi {
        id: "live".into(),
        base_url: cc_bridge_lib::proxy::custom_api::normalize_base_url(&base).unwrap(),
        api_key: key.clone(),
        model: MODEL.into(),
        auth_style: report.auth_style.clone().unwrap_or(AuthStyle::XApiKey),
        verified_at: Some(1),
    }]);
    let port = free_port();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(serve_with_shutdown(
        ServerConfig { bind_address: "127.0.0.1".into(), port, monitor: None },
        async move {
            let _ = stop_rx.await;
        },
    ));
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let claude_config = dir.join("claude-config");
    let picker = format!("api-{MODEL}");
    let mut cmd = std::process::Command::new(CLAUDE_EXE);
    cmd.args(["-p", "--model", &picker, "say ok"])
        .env("ANTHROPIC_BASE_URL", format!("http://127.0.0.1:{port}"))
        .env("ANTHROPIC_API_KEY", "sk-ant-dummy-not-a-real-key")
        .env("CLAUDE_CONFIG_DIR", &claude_config)
        .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
        .env("ANTHROPIC_DEFAULT_HAIKU_MODEL", &picker)
        .env("NO_PROXY", "127.0.0.1,localhost")
        .env_remove("ANTHROPIC_AUTH_TOKEN")
        .env_remove("UCLOUD_KEY")
        .env_remove("DEEPSEEK_API_KEY")
        .env_remove("CUSTOM_API_KEY_VAR")
        .stdin(std::process::Stdio::null());
    let out = tokio::time::timeout(
        Duration::from_secs(180),
        tokio::task::spawn_blocking(move || cmd.output()),
    )
    .await
    .expect("claude.exe 超时")
    .unwrap()
    .expect("启动 claude.exe 失败");
    let stdout = String::from_utf8_lossy(&out.stdout).replace(&key, "***");
    let stderr = String::from_utf8_lossy(&out.stderr).replace(&key, "***");
    println!("[e2e] exit={:?} stdout={:?} stderr={:?}", out.status.code(), stdout.trim(), stderr.trim());
    let _ = stop_tx.send(());
    let _ = server.await;
    set_entries(vec![]);

    // 泄露检查：状态目录（proxy.log、traffic）、CCP 配置目录、CLAUDE_CONFIG_DIR、日志回调、claude 输出
    let (files, file_hits) = count_hits(&dir, &tail);
    let log_lines = logs.lock().unwrap().clone();
    let log_hits: usize = log_lines.iter().map(|l| l.matches(&tail).count()).sum();
    let out_hits = String::from_utf8_lossy(&out.stdout).matches(&tail).count()
        + String::from_utf8_lossy(&out.stderr).matches(&tail).count();
    let traffic = count_hits(&dir.join("state").join("claude-code-proxy").join("traffic"), "").0
        + count_hits(&dir.join("state").join("traffic"), "").0;
    println!(
        "[leak] scanned_files={files} traffic_files={traffic} log_lines={} hits: files={file_hits} logs={log_hits} claude_output={out_hits}",
        log_lines.len()
    );

    assert_eq!(file_hits + log_hits + out_hits, 0, "key 后 8 位出现在日志/文件/输出中");
    assert!(out.status.success(), "claude -p 失败");
    assert!(stdout.to_ascii_lowercase().contains("ok"), "回答里没有 ok：{stdout}");
    assert!(report.ok, "真 Claude 中转校验未通过：{verify_desc}");
}

#[tokio::test]
#[ignore = "需要 DEEPSEEK_API_KEY 与网络"]
async fn deepseek_posing_as_claude_is_rejected() {
    sandbox();
    let key = secret("DEEPSEEK_API_KEY");
    let base = env_or("DEEPSEEK_BASE", "https://api.deepseek.com/anthropic");
    let report = verify(&http_client(), &base, &key, MODEL, |_, _| {}).await;
    println!("[deepseek verify] ok={} auth={:?} {}", report.ok, report.auth_style, describe(&report));
    assert!(!report.ok, "非 Claude 接口不应通过校验");
}
