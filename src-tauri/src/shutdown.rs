//! 系统关机 / 注销 / 终止信号时撤销接管并结束代理。
use crate::controller::Controller;
use std::sync::{Arc, OnceLock};

static CTL: OnceLock<Arc<Controller>> = OnceLock::new();

pub fn install(ctl: Arc<Controller>, window: &tauri::WebviewWindow) {
    let _ = CTL.set(ctl);
    #[cfg(windows)]
    win::hook(window);
    #[cfg(unix)]
    {
        let _ = window;
        unix::hook();
    }
}

fn on_session_end() {
    if let Some(c) = CTL.get() {
        c.shutdown();
    }
}

#[cfg(windows)]
mod win {
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        UI::{Shell::{DefSubclassProc, SetWindowSubclass}, WindowsAndMessaging::WM_ENDSESSION},
    };

    unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _id: usize, _data: usize) -> LRESULT {
        if msg == WM_ENDSESSION && wp != 0 {
            super::on_session_end();
        }
        unsafe { DefSubclassProc(hwnd, msg, wp, lp) }
    }

    pub fn hook(window: &tauri::WebviewWindow) {
        if let Ok(h) = window.hwnd() {
            unsafe {
                SetWindowSubclass(h.0 as HWND, Some(proc), 0xC0DE, 0);
            }
        }
    }
}

#[cfg(unix)]
mod unix {
    use signal_hook::{consts::{SIGHUP, SIGINT, SIGTERM}, iterator::Signals};

    pub fn hook() {
        if let Ok(mut s) = Signals::new([SIGTERM, SIGINT, SIGHUP]) {
            std::thread::spawn(move || {
                if s.forever().next().is_some() {
                    super::on_session_end();
                    std::process::exit(0);
                }
            });
        }
    }
}
