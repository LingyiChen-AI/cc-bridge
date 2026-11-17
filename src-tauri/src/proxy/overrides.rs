//! 进程内运行时覆盖（Codex Bridge 新增）。
//!
//! 代理原本通过环境变量读取 HOME、Codex 登录文件位置和出口代理。嵌入到桌面客户端后，
//! 客户端在启动代理前调用 [`set`] 显式传入这些值，代理优先使用这里的值，
//! 不再依赖（也不修改）进程环境变量。未设置时行为与原来完全一致。
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RuntimeOverrides {
    /// 用户主目录（代替环境变量 HOME / USERPROFILE）
    pub home: Option<PathBuf>,
    /// Codex CLI 登录文件（代替 CCP_CODEX_AUTH_FILE / CODEX_HOME / HOME 推导）
    pub codex_auth_file: Option<PathBuf>,
    /// 出口代理；Some 时同时作为 HTTP 与 HTTPS 代理，并忽略 HTTP(S)_PROXY / ALL_PROXY 环境变量
    pub outbound_proxy: Option<String>,
    /// 出口代理例外；仅在 outbound_proxy 设置时生效
    pub no_proxy: Option<String>,
    /// true 表示「明确直连」：即使环境变量里有代理也不使用
    pub direct: bool,
}

static OVERRIDES: RwLock<Option<RuntimeOverrides>> = RwLock::new(None);

type Sink = Arc<dyn Fn(&str) + Send + Sync>;
static LOG_SINK: OnceLock<RwLock<Option<Sink>>> = OnceLock::new();

pub fn set(overrides: RuntimeOverrides) {
    *OVERRIDES.write().unwrap_or_else(|e| e.into_inner()) = Some(overrides);
}

pub fn clear() {
    *OVERRIDES.write().unwrap_or_else(|e| e.into_inner()) = None;
}

pub fn get() -> Option<RuntimeOverrides> {
    OVERRIDES.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// 每条结构化日志（JSON 行）额外交给这个回调，供客户端界面展示。
pub fn set_log_sink(sink: impl Fn(&str) + Send + Sync + 'static) {
    let cell = LOG_SINK.get_or_init(|| RwLock::new(None));
    *cell.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(sink));
}

pub(crate) fn emit_log(line: &str) {
    if let Some(cell) = LOG_SINK.get() {
        let sink = cell.read().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(sink) = sink {
            sink(line);
        }
    }
}
