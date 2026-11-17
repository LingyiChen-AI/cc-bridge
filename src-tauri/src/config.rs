//! 客户端配置与接管记录（存放在 app data 目录）。
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{fs, io, path::{Path, PathBuf}};

/// 开发构建（tauri dev）与正式版隔离：不同端口、不同数据目录、只写沙盒 settings。
pub const DEV_MODE: bool = cfg!(debug_assertions);
pub const DEFAULT_MODEL_COUNT: usize = 6;
pub const DEFAULT_PORT: u16 = if DEV_MODE { 18766 } else { 18765 };

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct AppConfig {
    pub port: u16,
    pub outbound_proxy: Option<String>,
    pub codex_home: Option<PathBuf>,
    pub claude_dir: Option<PathBuf>,
    pub autostart: bool,
    pub takeover: bool,
    /// 写进 Claude Code 选单的模型；None = 默认取最新的 DEFAULT_MODEL_COUNT 个
    pub selected_models: Option<Vec<String>>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self { port: DEFAULT_PORT, outbound_proxy: None, codex_home: None, claude_dir: None, autostart: true, takeover: true, selected_models: None }
    }
}

impl AppConfig {
    /// 实际写入的模型：按可用列表（新→旧）顺序，过滤掉已不存在的选择。
    pub fn effective_models(&self, available: &[String]) -> Vec<String> {
        match &self.selected_models {
            None => available.iter().take(DEFAULT_MODEL_COUNT).cloned().collect(),
            Some(sel) => available.iter().filter(|m| sel.contains(m)).cloned().collect(),
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn effective_models_default_and_selected() {
        let avail: Vec<String> = (1..=8).rev().map(|i| format!("gpt-{i}")).collect();
        let mut cfg = AppConfig::default();
        assert_eq!(cfg.effective_models(&avail), avail[..6].to_vec());
        cfg.selected_models = Some(vec!["gpt-1".into(), "gpt-8".into(), "gone".into()]);
        assert_eq!(cfg.effective_models(&avail), ["gpt-8", "gpt-1"]);
    }
}
