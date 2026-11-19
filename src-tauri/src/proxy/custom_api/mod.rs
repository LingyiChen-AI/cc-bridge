//! 自定义 Claude API：用户自带 base URL / key / model，在 Claude Code `/model` 里显示为 `api-<model>`。
//! 本模块只含数据模型、校验规则与进程内全局路由表（客户端经 `set_entries` 整体替换）。
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

/// 模型名：1～128 个字符，首字符为字母或数字，其余只能是字母、数字和 `. _ : / -`
/// （覆盖 `qwen3-max`、`Qwen/Qwen3-Coder`、`kimi-k2:latest` 等写法；不许空格、引号、`[` 等，
/// 后者会和 Claude Code 的 `[1m]` 后缀冲突）。
pub fn validate_model(model: &str) -> Result<(), String> {
    let b = model.as_bytes();
    let ok = (1..=128).contains(&b.len())
        && b[0].is_ascii_alphanumeric()
        && b[1..].iter().all(|c| c.is_ascii_alphanumeric() || b"._:/-".contains(c));
    if ok {
        Ok(())
    } else {
        Err(format!(
            "模型名需为 1～128 个字符，以字母或数字开头，只能包含字母、数字和 . _ : / -：{model}"
        ))
    }
}

/// 去首尾空白、结尾 `/`、结尾 `/v1`；必须以 http:// 或 https:// 开头，不许带 userinfo。
pub fn normalize_base_url(raw: &str) -> Result<String, String> {
    let mut url = raw.trim().trim_end_matches('/');
    if let Some(stripped) = url.strip_suffix("/v1/messages") {
        url = stripped.trim_end_matches('/');
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("Base URL 必须以 http:// 或 https:// 开头".into());
    }
    // 带 userinfo 的地址会把凭证带进 settings.json 的模型说明与前端列表，一律拒绝。
    let authority = url
        .split_once("://")
        .map_or(url, |(_, rest)| rest)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("");
    if authority.contains('@') {
        return Err("请不要在地址里带账号密码，把凭证填在 API Key 里".into());
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
/// 判重不区分大小写（`lookup` 也不区分），并跳过已被别的条目占用的名字（如模型本身就叫 `x-2`）。
pub fn picker_names(entries: &[CustomApi]) -> Vec<(String, CustomApi)> {
    let mut taken: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out = Vec::new();
    for e in entries.iter().filter(|e| e.verified_at.is_some()) {
        let base = format!("api-{}", e.model);
        let mut name = base.clone();
        let mut n = 1;
        while !taken.insert(name.to_ascii_lowercase()) {
            n += 1;
            name = format!("{base}-{n}");
        }
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
        .find(|(n, _)| n.eq_ignore_ascii_case(&name))
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
    fn validate_model_accepts_compatible_ids() {
        let (ok_long, bad_long) = ("m".repeat(128), "m".repeat(129));
        for ok in [
            "claude-opus-5-5",
            "claude-haiku-4-5-20251001",
            "qwen3-max",
            "deepseek-chat",
            "Qwen/Qwen3-Coder",
            "kimi-k2:latest",
            "us.anthropic.claude-opus-5-5-v1:0",
            "glm_4.6",
            "a",
            ok_long.as_str(),
        ] {
            assert!(validate_model(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "claude opus",
            " qwen3-max",
            "qwen\"max",
            "qwen'max",
            "claude-opus-5-5[1m]",
            "claude-opus-5-5@20260101",
            "-qwen",
            "/qwen",
            "模型",
            bad_long.as_str(),
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
    fn normalize_base_url_rejects_userinfo() {
        const MSG: &str = "请不要在地址里带账号密码，把凭证填在 API Key 里";
        for bad in [
            "https://user:pass@api.x.com",
            "https://user@api.x.com/v1",
            "http://a:b@127.0.0.1:8080/proxy",
            " https://sk-secret@api.x.com/ ",
        ] {
            assert_eq!(normalize_base_url(bad).unwrap_err(), MSG, "{bad}");
        }
        // 路径 / 查询里的 @ 不是 userinfo
        assert_eq!(normalize_base_url("https://x.com/@team/anthropic").unwrap(), "https://x.com/@team/anthropic");
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

    fn verified(id: &str, model: &str) -> CustomApi {
        CustomApi {
            id: id.into(),
            base_url: "https://a".into(),
            api_key: "k".into(),
            model: model.into(),
            auth_style: AuthStyle::XApiKey,
            verified_at: Some(1),
        }
    }

    #[test]
    fn picker_names_unique_ignoring_case_and_collisions() {
        let names: Vec<String> = picker_names(&[
            verified("1", "Qwen/Qwen3-Coder"),
            verified("2", "qwen/qwen3-coder"),
            verified("3", "x"),
            verified("4", "x-2"),
            verified("5", "x"),
        ])
        .into_iter()
        .map(|(n, _)| n)
        .collect();
        assert_eq!(names, ["api-Qwen/Qwen3-Coder", "api-qwen/qwen3-coder-2", "api-x", "api-x-2", "api-x-3"]);
    }

    #[test]
    fn lookup_handles_slash_colon_and_case() {
        let _g = lock();
        set_entries(vec![
            verified("1", "Qwen/Qwen3-Coder"),
            verified("2", "kimi-k2:latest"),
            verified("3", "deepseek-chat"),
        ]);
        assert_eq!(lookup("api-Qwen/Qwen3-Coder").unwrap().id, "1");
        assert_eq!(lookup("api-qwen/qwen3-coder[1M]").unwrap().id, "1");
        assert_eq!(lookup("api-kimi-k2:latest[1m]").unwrap().id, "2");
        assert_eq!(lookup("api-deepseek-chat").unwrap().model, "deepseek-chat");
        assert!(lookup("api-kimi-k2").is_none());
        assert_eq!(picker_list(), ["api-Qwen/Qwen3-Coder", "api-kimi-k2:latest", "api-deepseek-chat"]);
        set_entries(vec![]);
    }
}
