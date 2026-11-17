//! 平台相关的小工具：系统代理检测、codex CLI 检测、打开终端执行 codex login、出口连通测试。
use std::{process::Command, time::{Duration, Instant}};

pub fn codex_cli_version() -> Option<String> {
    let path = which::which("codex").ok()?;
    let mut cmd = Command::new(path);
    cmd.arg("--version");
    hide_window(&mut cmd);
    let out = cmd.output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Some(if s.is_empty() { "已安装".into() } else { s })
}

fn hide_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

/// 系统代理，统一成 `http://host:port`。
pub fn system_proxy() -> Option<String> {
    #[cfg(windows)]
    {
        use winreg::{RegKey, enums::HKEY_CURRENT_USER};
        let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings").ok()?;
        let enabled: u32 = key.get_value("ProxyEnable").unwrap_or(0);
        if enabled == 1 {
            let server: String = key.get_value("ProxyServer").ok()?;
            return normalize_proxy(&server);
        }
    }
    ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"]
        .iter()
        .find_map(|k| std::env::var(k).ok())
        .and_then(|v| normalize_proxy(&v))
}

/// 支持 `host:port`、`http://host:port`、`http=h:p;https=h2:p2`（取 https，否则 http）。
pub fn normalize_proxy(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let pick = if raw.contains('=') {
        let parts: Vec<(&str, &str)> = raw.split(';').filter_map(|p| p.split_once('=')).collect();
        parts.iter().find(|(k, _)| k.eq_ignore_ascii_case("https")).or_else(|| parts.iter().find(|(k, _)| k.eq_ignore_ascii_case("http")))?.1
    } else {
        raw
    };
    Some(if pick.contains("://") { pick.to_string() } else { format!("http://{pick}") })
}

/// 经出口代理访问 OpenAI 登录域名，返回耗时毫秒。任何 HTTP 响应都算连通。
pub fn test_outbound(proxy: Option<&str>) -> Result<u64, String> {
    let mut b = reqwest::blocking::Client::builder().timeout(Duration::from_secs(8));
    b = match proxy.map(str::trim).filter(|s| !s.is_empty()) {
        Some(p) => b.proxy(reqwest::Proxy::all(p).map_err(|e| format!("代理地址无效：{e}"))?),
        None => b.no_proxy(),
    };
    let client = b.build().map_err(|e| e.to_string())?;
    let t = Instant::now();
    client.get("https://auth.openai.com/").send().map_err(|e| {
        if e.is_timeout() { "连接超时".to_string() } else if e.is_connect() { "无法连接（检查代理是否开启）".to_string() } else { e.to_string() }
    })?;
    Ok(t.elapsed().as_millis() as u64)
}

pub fn open_codex_login() -> Result<(), String> {
    if which::which("codex").is_err() {
        return Err("未安装 Codex CLI：npm i -g @openai/codex".into());
    }
    #[cfg(windows)]
    let r = Command::new("cmd").args(["/c", "start", "Codex 登录", "cmd", "/k", "codex login"]).spawn();
    #[cfg(target_os = "macos")]
    let r = Command::new("osascript").args(["-e", "tell application \"Terminal\" to do script \"codex login\"", "-e", "tell application \"Terminal\" to activate"]).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = {
        let tries: [(&str, &[&str]); 3] = [("x-terminal-emulator", &["-e"]), ("gnome-terminal", &["--"]), ("konsole", &["-e"])];
        tries
            .iter()
            .find_map(|(bin, pre)| Command::new(bin).args(*pre).args(["sh", "-c", "codex login; exec $SHELL"]).spawn().ok())
            .ok_or_else(|| std::io::Error::other("没有找到可用的终端"))
    };
    r.map(|_| ()).map_err(|e| format!("打开终端失败：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalize_variants() {
        assert_eq!(normalize_proxy("127.0.0.1:7890").as_deref(), Some("http://127.0.0.1:7890"));
        assert_eq!(normalize_proxy("http=a:1;https=b:2").as_deref(), Some("http://b:2"));
        assert_eq!(normalize_proxy("socks5://c:3").as_deref(), Some("socks5://c:3"));
        assert_eq!(normalize_proxy(" "), None);
    }
}
