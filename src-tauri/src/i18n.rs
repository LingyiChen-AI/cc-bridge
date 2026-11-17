//! 后端文案（托盘菜单、系统通知）的中英文。界面文案在前端 src/i18n.ts。

pub fn tr(lang: &str, key: &str) -> &'static str {
    let zh = lang == "zh";
    match key {
        "trayProxyRunning" => if zh { "代理运行中" } else { "Proxy running" },
        "trayProxyStarting" => if zh { "代理启动中" } else { "Proxy starting" },
        "trayProxyRestarting" => if zh { "代理重启中" } else { "Proxy restarting" },
        "trayProxyFailed" => if zh { "代理已停止：反复崩溃" } else { "Proxy stopped: crashed repeatedly" },
        "trayPortBusy" => if zh { "端口被占用" } else { "Port in use" },
        "trayProxyStopped" => if zh { "代理未启动" } else { "Proxy not started" },
        "trayAuth401" => if zh { "Codex 登录无效（401）" } else { "Codex login invalid (401)" },
        "trayAuthOk" => if zh { "Codex 已登录" } else { "Codex signed in" },
        "trayAuthRefresh" => if zh { "Codex 令牌待续期" } else { "Codex token pending refresh" },
        "trayAuthMissing" => if zh { "Codex 未登录" } else { "Codex not signed in" },
        "trayTaken" => if zh { "Claude Code 已接管" } else { "Claude Code connected" },
        "trayWaiting" => if zh { "Claude Code 等待接管" } else { "Claude Code waiting" },
        "trayNotTaken" => if zh { "Claude Code 未接管" } else { "Claude Code not connected" },
        "menuOpen" => if zh { "打开主窗口" } else { "Open window" },
        "menuTakeover" => if zh { "接管 Claude Code" } else { "Connect Claude Code" },
        "menuRestart" => if zh { "重启代理" } else { "Restart proxy" },
        "menuAutostart" => if zh { "开机自启" } else { "Launch at login" },
        "menuQuit" => if zh { "退出" } else { "Quit" },
        "notifyFailedTitle" => if zh { "CC Bridge：代理不可用" } else { "CC Bridge: proxy unavailable" },
        "notifyFailedBody" => if zh {
            "已临时撤销对 Claude Code 的接管，新开的 Claude Code 窗口只用 Claude 模型。"
        } else {
            "Claude Code has been disconnected for now; new Claude Code windows will use Claude models only."
        },
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::tr;
    #[test]
    fn every_key_has_both_languages() {
        for k in ["trayProxyRunning", "menuQuit", "notifyFailedBody", "trayAuthMissing"] {
            assert!(!tr("zh", k).is_empty() && !tr("en", k).is_empty());
            assert_ne!(tr("zh", k), tr("en", k));
        }
    }
}
