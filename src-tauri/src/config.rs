//! 客户端配置与接管记录（存放在 app data 目录）。
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{fs, io, path::{Path, PathBuf}};

pub const DEFAULT_PORT: u16 = 18765;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct AppConfig {
    pub port: u16,
    pub outbound_proxy: Option<String>,
    pub codex_home: Option<PathBuf>,
    pub claude_dir: Option<PathBuf>,
    pub autostart: bool,
    pub takeover: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self { port: DEFAULT_PORT, outbound_proxy: None, codex_home: None, claude_dir: None, autostart: true, takeover: true }
    }
}

impl AppConfig {
    /// 出口代理：空字符串视为未配置。
    pub fn outbound(&self) -> Option<&str> {
        self.outbound_proxy.as_deref().map(str::trim).filter(|s| !s.is_empty())
    }
    pub fn claude_settings_path(&self, home: &Path) -> PathBuf {
        self.claude_dir.clone().unwrap_or_else(|| home.join(".claude")).join("settings.json")
    }
}

/// 本客户端写入 settings.json 的内容，撤销时据此只删自己写的。
/// `prev_model` / `prev_env_model`：外层 None = 未记录；Some(None) = 接管前不存在该字段。
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct TakeoverRecord {
    pub base_url: Option<String>,
    pub models: Vec<String>,
    pub prev_model: Option<Option<String>>,
    pub prev_env_model: Option<Option<String>>,
    /// 接管前不存在 `env` / `modelPicker`，撤销时变空就整个删掉。
    pub created_env: bool,
    pub created_picker: bool,
}

pub fn load_json<T: DeserializeOwned + Default>(path: &Path) -> T {
    fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save_json<T: Serialize>(path: &Path, v: &T) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(v).map_err(io::Error::other)?;
    write_atomic(path, &bytes)
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}
