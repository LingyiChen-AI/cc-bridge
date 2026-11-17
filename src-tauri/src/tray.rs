//! 系统托盘：三行状态 + 常用操作。
use crate::{
    auth_status::AuthState,
    controller::{Controller, StatusDto},
    supervisor::ProxyState,
};
use std::sync::Arc;
use tauri::{
    AppHandle, Manager, Wry,
    image::Image,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
};
use tauri_plugin_autostart::ManagerExt;
use crate::i18n::tr;

pub struct Tray {
    icon: TrayIcon,
    open: MenuItem<Wry>,
    restart: MenuItem<Wry>,
    quit: MenuItem<Wry>,
    proxy: MenuItem<Wry>,
    auth: MenuItem<Wry>,
    takeover_line: MenuItem<Wry>,
    takeover: CheckMenuItem<Wry>,
    autostart: CheckMenuItem<Wry>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Level {
    Ok,
    Warn,
    Err,
}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

pub fn build(app: &AppHandle, ctl: Arc<Controller>) -> tauri::Result<Tray> {
    let l = ctl.config().lang();
    let proxy = MenuItem::with_id(app, "s_proxy", tr(l, "trayProxyStarting"), false, None::<&str>)?;
    let auth = MenuItem::with_id(app, "s_auth", "Codex", false, None::<&str>)?;
    let takeover_line = MenuItem::with_id(app, "s_take", tr(l, "trayNotTaken"), false, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", tr(l, "menuOpen"), true, None::<&str>)?;
    let takeover = CheckMenuItem::with_id(app, "takeover", tr(l, "menuTakeover"), true, ctl.config().takeover, None::<&str>)?;
    let restart = MenuItem::with_id(app, "restart", tr(l, "menuRestart"), true, None::<&str>)?;
    let autostart = CheckMenuItem::with_id(app, "autostart", tr(l, "menuAutostart"), true, app.autolaunch().is_enabled().unwrap_or(false), None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", tr(l, "menuQuit"), true, None::<&str>)?;
    let sep = || PredefinedMenuItem::separator(app);
    let menu = Menu::with_items(app, &[&proxy, &auth, &takeover_line, &sep()?, &open, &takeover, &restart, &autostart, &sep()?, &quit])?;

    let c = ctl.clone();
    let icon = TrayIconBuilder::with_id("main")
        .icon(level_icon(Level::Warn))
        .tooltip("CC Bridge")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, e| match e.id().as_ref() {
            "open" => show_main(app),
            "takeover" => c.set_takeover(!c.config().takeover),
            "restart" => c.sup.restart(),
            "autostart" => {
                let mut cfg = c.config();
                cfg.autostart = !cfg.autostart;
                crate::apply_autostart(app, cfg.autostart);
                c.save_config(cfg);
            }
            "quit" => crate::quit(app, &c),
            _ => {}
        })
        .on_tray_icon_event(|tray, e| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                show_main(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(Tray { icon, open, restart, quit, proxy, auth, takeover_line, takeover, autostart })
}

pub fn level_icon(l: Level) -> Image<'static> {
    let bytes: &'static [u8] = match l {
        Level::Ok => include_bytes!("../icons/tray-ok.png"),
        Level::Warn => include_bytes!("../icons/tray-warn.png"),
        Level::Err => include_bytes!("../icons/tray-err.png"),
    };
    Image::from_bytes(bytes).expect("tray icon")
}

pub fn proxy_text(st: &StatusDto) -> (Level, String) {
    let l = st.lang.as_str();
    match &st.proxy {
        ProxyState::Running => (Level::Ok, format!("{} :{}", tr(l, "trayProxyRunning"), st.port)),
        ProxyState::Starting => (Level::Warn, tr(l, "trayProxyStarting").into()),
        ProxyState::Restarting { attempt } => (Level::Warn, format!("{} ({attempt})", tr(l, "trayProxyRestarting"))),
        ProxyState::Failed { .. } => (Level::Err, tr(l, "trayProxyFailed").into()),
        ProxyState::PortBusy { .. } => (Level::Err, format!("{} :{}", tr(l, "trayPortBusy"), st.port)),
        ProxyState::Stopped => (Level::Warn, tr(l, "trayProxyStopped").into()),
    }
}

pub fn auth_text(st: &StatusDto) -> (Level, String) {
    let l = st.lang.as_str();
    if st.auth_hint.is_some() {
        return (Level::Err, tr(l, "trayAuth401").into());
    }
    match &st.auth {
        AuthState::Ok { .. } => (Level::Ok, tr(l, "trayAuthOk").into()),
        AuthState::ExpiredRefreshable => (Level::Warn, tr(l, "trayAuthRefresh").into()),
        AuthState::Missing { .. } => (Level::Err, tr(l, "trayAuthMissing").into()),
    }
}

impl Tray {
    pub fn update(&self, st: &StatusDto) {
        let (pl, pt) = proxy_text(st);
        let (al, at) = auth_text(st);
        let l = st.lang.as_str();
        let take = tr(l, if st.taken_over { "trayTaken" } else if st.takeover { "trayWaiting" } else { "trayNotTaken" });
        // 语言可能在设置里切换，菜单文字每次同步
        let _ = self.open.set_text(tr(l, "menuOpen"));
        let _ = self.restart.set_text(tr(l, "menuRestart"));
        let _ = self.quit.set_text(tr(l, "menuQuit"));
        let _ = self.takeover.set_text(tr(l, "menuTakeover"));
        let _ = self.autostart.set_text(tr(l, "menuAutostart"));
        let dot = |l: Level| match l {
            Level::Ok => "●",
            Level::Warn => "◐",
            Level::Err => "○",
        };
        let _ = self.proxy.set_text(format!("{} {pt}", dot(pl)));
        let _ = self.auth.set_text(format!("{} {at}", dot(al)));
        let _ = self.takeover_line.set_text(format!("{} {take}", if st.taken_over { "●" } else { "○" }));
        let _ = self.takeover.set_checked(st.takeover);
        let _ = self.autostart.set_checked(st.config.autostart);
        let overall = if pl == Level::Err || al == Level::Err || st.settings_error.is_some() {
            Level::Err
        } else if pl == Level::Warn || al == Level::Warn || (st.takeover && !st.taken_over) {
            Level::Warn
        } else {
            Level::Ok
        };
        let _ = self.icon.set_icon(Some(level_icon(overall)));
        let _ = self.icon.set_tooltip(Some(format!("CC Bridge\n{pt}\n{at}\n{take}")));
    }
}
