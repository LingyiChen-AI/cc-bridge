//! 自定义 Claude API：用户自带 base URL / key / model，在 Claude Code `/model` 里显示为 `api-<model>`。
//! 本模块只含数据模型、校验规则与进程内全局路由表（客户端经 `set_entries` 整体替换）。
pub mod fingerprint;
pub mod provider;
pub mod verify;

use serde::{Deserialize, Serialize};
use std::sync::RwLock;

use super::registry::normalize_incoming_model;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AuthStyle {
    XApiKey,
    Bearer,
}

#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub struct CustomApi {
    pub id: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub auth_style: AuthStyle,
    pub verified_at: Option<u64>,
}

/// 手写 Debug：key 只以掩码形式出现，避免经 `{:?}` 泄漏进日志。
impl std::fmt::Debug for CustomApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CustomApi")
            .field("id", &self.id)
            .field("base_url", &self.base_url)
            .field("api_key", &mask_key(&self.api_key))
            .field("model", &self.model)
            .field("auth_style", &self.auth_style)
            .field("verified_at", &self.verified_at)
            .finish()
    }
}

/// 选择器名 -> 条目，只含已验证的条目。
static ENTRIES: RwLock<Vec<(String, CustomApi)>> = RwLock::new(Vec::new());

/// 写全局表的测试共用的锁（Task 2 / 4 的测试也要持有）。
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 模型名必须匹配 `^claude-[a-z0-9-]+$`。
pub fn validate_model(model: &str) -> Result<(), String> {
    let ok = model
        .strip_prefix("claude-")
        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'));
    if ok {
        Ok(())
    } else {
        Err(format!("模型名需为官方 Claude 模型 ID（claude-xxx，仅小写字母、数字、连字符）：{model}"))
    }
}

/// 去首尾空白、结尾 `/`、结尾 `/v1`；必须以 http:// 或 https:// 开头。
pub fn normalize_base_url(raw: &str) -> Result<String, String> {
    let mut url = raw.trim().trim_end_matches('/');
    if let Some(stripped) = url.strip_suffix("/v1/messages") {
        url = stripped.trim_end_matches('/');
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("Base URL 必须以 http:// 或 https:// 开头".into());
    }
    if let Some(stripped) = url.strip_suffix("/v1") {
        url = stripped.trim_end_matches('/');
    }
    Ok(url.to_string())
}

/// 前 7 位 + `…` + 后 4 位；长度 ≤ 12 时只显示 `…` + 后 2 位。
pub fn mask_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 12 {
        let tail: String = chars[chars.len().saturating_sub(2)..].iter().collect();
        format!("…{tail}")
    } else {
        let head: String = chars[..7].iter().collect();
        let tail: String = chars[chars.len() - 4..].iter().collect();
        format!("{head}…{tail}")
    }
}

/// 已验证条目按顺序生成 `api-<model>`，重名依次加 `-2`、`-3`…
pub fn picker_names(entries: &[CustomApi]) -> Vec<(String, CustomApi)> {
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    let mut out = Vec::new();
    for e in entries.iter().filter(|e| e.verified_at.is_some()) {
        let n = counts.entry(e.model.as_str()).or_insert(0);
        *n += 1;
        let name = if *n == 1 {
            format!("api-{}", e.model)
        } else {
            format!("api-{}-{}", e.model, n)
        };
        out.push((name, e.clone()));
    }
    out
}

pub fn set_entries(entries: Vec<CustomApi>) {
    let named = picker_names(&entries);
    *ENTRIES.write().unwrap_or_else(|e| e.into_inner()) = named;
}

pub fn lookup(picker_name: &str) -> Option<CustomApi> {
    let name = normalize_incoming_model(picker_name);
    ENTRIES
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, e)| e.clone())
}

pub fn picker_list() -> Vec<String> {
    ENTRIES
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .map(|(n, _)| n.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn validate_model_accepts_official_ids() {
        for ok in ["claude-opus-5-5", "claude-haiku-4-5-20251001", "claude-fable-5-1"] {
            assert!(validate_model(ok).is_ok(), "{ok}");
        }
        for bad in [
            "us.anthropic.claude-opus-5-5-v1:0",
            "claude-opus-5-5@20260101",
            "qwen3-max",
            "Claude-Opus",
            "claude-opus 5",
            "",
        ] {
            assert!(validate_model(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn normalize_base_url_variants() {
        for raw in [
            "https://api.x.com",
            "https://api.x.com/",
            "https://api.x.com/v1",
            " https://api.x.com/v1/ ",
            "https://api.x.com/v1/messages",
            "https://api.x.com/v1/messages/",
        ] {
            assert_eq!(normalize_base_url(raw).unwrap(), "https://api.x.com");
        }
        assert_eq!(
            normalize_base_url("https://x.com/proxy/anthropic").unwrap(),
            "https://x.com/proxy/anthropic"
        );
        assert!(normalize_base_url("api.x.com").is_err());
    }

    #[test]
    fn mask_key_hides_middle() {
        assert_eq!(mask_key("sk-ant-api03-abcdefghijklmnop1234"), "sk-ant-…1234");
        assert_eq!(mask_key("short123"), "…23");
    }

    #[test]
    fn picker_names_skip_unverified_and_number_duplicates() {
        let e = |id: &str, m: &str, v: bool| CustomApi {
            id: id.into(),
            base_url: "https://a".into(),
            api_key: "k".into(),
            model: m.into(),
            auth_style: AuthStyle::XApiKey,
            verified_at: v.then_some(1),
        };
        let names: Vec<String> = picker_names(&[
            e("1", "claude-opus-5-5", true),
            e("2", "claude-opus-5-5", false),
            e("3", "claude-opus-5-5", true),
            e("4", "claude-sonnet-5-5", true),
        ])
        .into_iter()
        .map(|(n, _)| n)
        .collect();
        assert_eq!(
            names,
            ["api-claude-opus-5-5", "api-claude-opus-5-5-2", "api-claude-sonnet-5-5"]
        );
    }

    #[test]
    fn lookup_strips_1m_suffix() {
        let _g = lock();
        set_entries(vec![CustomApi {
            id: "1".into(),
            base_url: "https://a".into(),
            api_key: "k".into(),
            model: "claude-opus-5-5".into(),
            auth_style: AuthStyle::XApiKey,
            verified_at: Some(1),
        }]);
        assert_eq!(lookup("api-claude-opus-5-5[1m]").unwrap().id, "1");
        assert_eq!(picker_list(), ["api-claude-opus-5-5"]);
        assert!(lookup("claude-opus-5-5").is_none());
        set_entries(vec![]);
    }
}
