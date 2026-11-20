//! 平台相关的小工具：系统代理检测、codex CLI 检测、打开终端执行 codex login、出口连通测试。
use std::{path::PathBuf, process::Command, time::{Duration, Instant}};

/// 定位 codex 可执行文件。
pub fn find_codex() -> Option<PathBuf> {
    find_bin("codex")
}

/// 定位命令行工具（codex、claude）。
///
/// macOS / Linux 上不能只看进程 PATH：从 Finder、Dock、开机自启起来的 .app 继承的是 launchd
/// 的最小 PATH（`/usr/bin:/bin:/usr/sbin:/sbin`），不会读 `.zshrc`，于是装在 `~/.local/bin`、
/// nvm、homebrew 下的 codex 一律找不到——从终端跑的开发版却能找到，正是这个差别。
/// 这里拿登录 shell 的 PATH 加上常见安装目录一起找。
/// Windows 上 GUI 进程继承的是 Explorer 启动时的 PATH，装完工具没重新登录就看不到；
/// 再补上 Claude Code 原生安装器（`~\.local\bin`）和 npm 全局（`%APPDATA%\npm`）两个目录。
pub fn find_bin(name: &str) -> Option<PathBuf> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    which::which_in(name, Some(search_path()), cwd).ok()
}

#[cfg(windows)]
fn search_path() -> &'static std::ffi::OsString {
    use std::sync::OnceLock;
    static CACHE: OnceLock<std::ffi::OsString> = OnceLock::new();
    CACHE.get_or_init(|| {
        let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
        if let Some(home) = dirs::home_dir() {
            dirs.push(home.join(".local").join("bin"));
        }
        if let Some(appdata) = std::env::var_os("APPDATA") {
            dirs.push(PathBuf::from(appdata).join("npm"));
        }
        let mut seen = Vec::new();
        dirs.retain(|d| {
            let k = d.to_string_lossy().to_lowercase();
            !seen.contains(&k) && {
                seen.push(k);
                true
            }
        });
        std::env::join_paths(dirs).unwrap_or_default()
    })
}

/// 选单里列 Codex 模型靠 settings.json 的 `modelPicker`，Claude Code 2.1.243 才开始支持；
/// 更早的版本会静默忽略，`/model` 里只有 Claude 自己的模型。
pub const MIN_CLAUDE_CODE: (u32, u32, u32) = (2, 1, 243);

/// 本机 Claude Code 命令行的版本号（`claude --version` 的 `2.1.329 (Claude Code)` 取前半）。
/// 没装或解析不出来返回 None。
pub fn claude_code_version() -> Option<String> {
    let path = find_bin("claude")?;
    let mut cmd = Command::new(path);
    cmd.arg("--version");
    hide_window(&mut cmd);
    let out = cmd.output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let (a, b, c) = parse_version(&s)?;
    Some(format!("{a}.{b}.{c}"))
}

/// 从输出里找第一个 `x.y.z`。
pub fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    s.split(|c: char| !(c.is_ascii_digit() || c == '.')).find_map(|tok| {
        let mut it = tok.split('.');
        let v = (it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?);
        Some(v)
    })
}

/// 查找用的 PATH：登录 shell 的 PATH + 本进程 PATH + 常见的用户级 bin 目录，去重保序。
/// 只在第一次用到时问一次 shell（要跑 rc 文件，有几百毫秒开销）。
#[cfg(unix)]
fn search_path() -> &'static std::ffi::OsString {
    use std::sync::OnceLock;
    static CACHE: OnceLock<std::ffi::OsString> = OnceLock::new();
    CACHE.get_or_init(|| {
        let home = dirs::home_dir().unwrap_or_default();
        let user_bins: Vec<String> = [".local/bin", ".bun/bin", ".volta/bin", ".yarn/bin", "Library/pnpm", ".npm-global/bin", ".deno/bin"]
            .iter()
            .map(|d| home.join(d).to_string_lossy().into_owned())
            .collect();
        let sources = [
            login_shell_path().unwrap_or_default(),
            std::env::var("PATH").unwrap_or_default(),
            user_bins.join(":"),
            "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin".to_string(),
        ];
        std::ffi::OsString::from(merge_paths(&sources))
    })
}

/// 依次串起若干 PATH 片段，去掉空项和重复项，保持先后顺序。
#[cfg(unix)]
fn merge_paths(sources: &[String]) -> String {
    let mut dirs: Vec<&str> = Vec::new();
    for raw in sources {
        for d in raw.split(':').filter(|s| !s.is_empty()) {
            if !dirs.contains(&d) {
                dirs.push(d);
            }
        }
    }
    dirs.join(":")
}

#[cfg(unix)]
const PATH_MARK: &str = "__CCB_PATH__";

/// 从 `$SHELL -ilc` 的输出里取出被标记夹住的 PATH —— rc 文件自己的输出（欢迎语、
/// 版本管理器的提示）会混在 stdout 里，不能整段拿来用。
#[cfg(unix)]
fn parse_marked_path(stdout: &str) -> Option<String> {
    let v = stdout.split(PATH_MARK).nth(1)?.trim().to_string();
    Some(v).filter(|v| !v.is_empty())
}

/// 跑一次登录 shell 把它的 PATH 问出来。用 `-i` 是因为 PATH 多半写在 `.zshrc` 里，
/// 而 `.zshrc` 只有交互式 shell 才读。卡住就放弃：单开线程跑，5 秒没结果就当没有。
#[cfg(unix)]
fn login_shell_path() -> Option<String> {
    // GUI 启动的进程往往连 SHELL 都没有；macOS 10.15 起默认 zsh，/bin/sh 不读 .zshrc，兜底要挑对
    let default_shell = if cfg!(target_os = "macos") { "/bin/zsh" } else { "/bin/sh" };
    let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| default_shell.into());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let out = Command::new(&shell)
            .args(["-ilc", &format!("printf '{PATH_MARK}%s{PATH_MARK}' \"$PATH\"")])
            .stdin(std::process::Stdio::null())
            .output();
        let _ = tx.send(out);
    });
    let out = rx.recv_timeout(Duration::from_secs(5)).ok()?.ok()?;
    parse_marked_path(&String::from_utf8_lossy(&out.stdout))
}

pub fn codex_cli_version() -> Option<String> {
    let path = find_codex()?;
    let mut cmd = Command::new(path);
    cmd.arg("--version");
    hide_window(&mut cmd);
    let out = cmd.output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Some(if s.is_empty() { "codex".into() } else { s })
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
        Some(p) => b.proxy(reqwest::Proxy::all(p).map_err(|e| format!("badProxy: {e}"))?),
        None => b.no_proxy(),
    };
    let client = b.build().map_err(|e| e.to_string())?;
    let t = Instant::now();
    // 测 Codex 实际调用的地址（GET 会返回 405，能拿到任何 HTTP 响应即算连通）
    client.get("https://chatgpt.com/backend-api/codex/responses").send().map_err(|e| {
        if e.is_timeout() { "timeout".to_string() } else if e.is_connect() { "connect".to_string() } else { format!("other: {e}") }
    })?;
    Ok(t.elapsed().as_millis() as u64)
}

pub fn open_codex_login() -> Result<(), String> {
    let Some(exe) = find_codex() else {
        return Err("noCli".into());
    };
    // 终端里的 PATH 未必和这里一致（GUI 启动尤其），直接用绝对路径
    #[cfg(windows)]
    let r = Command::new("cmd").args(["/c", "start", "codex login", "cmd", "/k", &format!("\"{}\" login", exe.display())]).spawn();
    #[cfg(unix)]
    let cmdline = format!("'{}' login", exe.to_string_lossy().replace('\'', r"'\''"));
    #[cfg(target_os = "macos")]
    let r = Command::new("osascript")
        .args([
            "-e",
            &format!("tell application \"Terminal\" to do script \"{}\"", cmdline.replace('\\', "\\\\").replace('"', "\\\"")),
            "-e",
            "tell application \"Terminal\" to activate",
        ])
        .spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = {
        let script = format!("{cmdline}; exec $SHELL");
        let tries: [(&str, &[&str]); 3] = [("x-terminal-emulator", &["-e"]), ("gnome-terminal", &["--"]), ("konsole", &["-e"])];
        tries
            .iter()
            .find_map(|(bin, pre)| Command::new(bin).args(*pre).args(["sh", "-c", &script]).spawn().ok())
            .ok_or_else(|| std::io::Error::other("no terminal emulator found"))
    };
    r.map(|_| ()).map_err(|e| format!("terminal: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    /// GUI 启动时进程 PATH 只有 /usr/bin:/bin:/usr/sbin:/sbin，查找用的 PATH 必须比它宽，
    /// 至少把登录 shell 的 PATH 和常见的用户级 bin 目录带上，否则 codex 一定找不到。
    #[cfg(unix)]
    #[test]
    fn search_path_covers_user_bins() {
        let p = search_path().to_string_lossy().to_string();
        let dirs: Vec<&str> = p.split(':').collect();
        assert!(dirs.contains(&"/usr/bin"));
        let home = dirs::home_dir().unwrap();
        for d in [".local/bin", ".bun/bin"] {
            assert!(dirs.contains(&home.join(d).to_string_lossy().as_ref()), "缺少 {d}：{p}");
        }
        assert_eq!(dirs.len(), dirs.iter().collect::<std::collections::HashSet<_>>().len(), "有重复目录：{p}");
    }

    #[test]
    fn parse_version_variants() {
        assert_eq!(parse_version("2.1.329 (Claude Code)\n"), Some((2, 1, 329)));
        assert_eq!(parse_version("claude 2.1.243"), Some((2, 1, 243)));
        assert_eq!(parse_version("v10.0.1-beta"), Some((10, 0, 1)));
        assert_eq!(parse_version("1.2 (Claude Code)"), None);
        assert_eq!(parse_version(""), None);
    }

    /// 门槛按数字比，不按字符串比：2.1.30 比 2.1.243 旧，2.2.0 比它新。
    #[test]
    fn min_claude_code_compares_numerically() {
        let ok = |s: &str| parse_version(s).unwrap() >= MIN_CLAUDE_CODE;
        assert!(!ok("2.1.220"));
        assert!(!ok("2.1.30"));
        assert!(ok("2.1.243"));
        assert!(ok("2.1.329"));
        assert!(ok("2.2.0"));
        assert!(ok("3.0.0"));
    }

    #[cfg(unix)]
    #[test]
    fn merge_paths_dedupes_and_keeps_order() {
        let got = merge_paths(&["/a:/b".into(), String::new(), "/b:/c".into(), "::/a:/d".into()]);
        assert_eq!(got, "/a:/b:/c:/d");
    }

    /// rc 文件会往 stdout 打东西（nvm / conda 的提示之类），只能取标记之间那一段。
    #[cfg(unix)]
    #[test]
    fn parse_marked_path_ignores_rc_noise() {
        let out = format!("Welcome!\nnvm: using v24\n{PATH_MARK}/usr/bin:/home/me/.local/bin{PATH_MARK}");
        assert_eq!(parse_marked_path(&out).as_deref(), Some("/usr/bin:/home/me/.local/bin"));
        assert_eq!(parse_marked_path("只有噪声，没有标记"), None);
        assert_eq!(parse_marked_path(&format!("{PATH_MARK}{PATH_MARK}")), None);
    }

    #[test]
    fn normalize_variants() {
        assert_eq!(normalize_proxy("127.0.0.1:7890").as_deref(), Some("http://127.0.0.1:7890"));
        assert_eq!(normalize_proxy("http=a:1;https=b:2").as_deref(), Some("http://b:2"));
        assert_eq!(normalize_proxy("socks5://c:3").as_deref(), Some("socks5://c:3"));
        assert_eq!(normalize_proxy(" "), None);
    }
}
