//! 自定义 Claude API 的客户端侧存储与前端 DTO。API key 只落在 `custom_apis.json`，不进 AppConfig、日志、settings.json，也不发给前端。
use crate::config::{load_json, save_json};
use crate::proxy::custom_api::{CustomApi, mask_key};
use serde::{Deserialize, Serialize};
use std::{io, path::Path};

const FILE: &str = "custom_apis.json";

pub fn load(data_dir: &Path) -> Vec<CustomApi> {
    load_json(&data_dir.join(FILE))
}

pub fn save(data_dir: &Path, v: &[CustomApi]) -> io::Result<()> {
    save_json(&data_dir.join(FILE), &v)
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CustomApiDto {
    pub id: String,
    pub base_url: String,
    pub key_mask: String,
    pub model: String,
    pub picker_name: Option<String>,
    pub verified_at: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomApiInput {
    pub id: Option<String>,
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
}

pub fn to_dtos(v: &[CustomApi]) -> Vec<CustomApiDto> {
    let names = crate::proxy::custom_api::picker_names(v);
    v.iter()
        .map(|e| CustomApiDto {
            id: e.id.clone(),
            base_url: e.base_url.clone(),
            key_mask: mask_key(&e.api_key),
            model: e.model.clone(),
            picker_name: names.iter().find(|(_, n)| n.id == e.id).map(|(name, _)| name.clone()),
            verified_at: e.verified_at,
        })
        .collect()
}

/// key 只允许可见 ASCII（0x21..=0x7E），否则放进请求头时会被静默丢弃。
pub fn validate_key(key: &str) -> Result<(), String> {
    if key.is_empty() {
        return Err("请填写 API Key".into());
    }
    if key.bytes().all(|b| (0x21..=0x7E).contains(&b)) {
        Ok(())
    } else {
        Err("API Key 含有空格、控制字符或非 ASCII 字符，请检查是否多复制了换行或中文".into())
    }
}

/// 保存校验结果：校验通过则新增 / 覆盖；未通过时新增条目照样保存（未验证），已存在的条目保持原样。
pub(crate) fn merge_edit(existing: &[CustomApi], candidate: CustomApi, ok: bool) -> Vec<CustomApi> {
    let exists = existing.iter().any(|e| e.id == candidate.id);
    if !ok && exists {
        return existing.to_vec();
    }
    let mut out = existing.to_vec();
    match out.iter_mut().find(|e| e.id == candidate.id) {
        Some(slot) => *slot = candidate,
        None => out.push(candidate),
    }
    out
}

pub(crate) fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::custom_api::{AuthStyle, TEST_LOCK, set_entries};

    pub(crate) fn api(id: &str, model: &str, key: &str, verified: Option<u64>) -> CustomApi {
        CustomApi {
            id: id.into(),
            base_url: "https://relay.example.com".into(),
            api_key: key.into(),
            model: model.into(),
            auth_style: AuthStyle::XApiKey,
            verified_at: verified,
        }
    }

    #[test]
    fn dto_never_contains_key() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let key = "sk-ant-abcdefghijklmnopqrstuvwxyz0123456789";
        let v = vec![api("a", "claude-opus-5-5", key, Some(1)), api("b", "claude-opus-5-5", key, None)];
        set_entries(v.clone());
        let dtos = to_dtos(&v);
        let s = serde_json::to_string(&dtos).unwrap();
        assert!(!s.contains(key));
        assert_eq!(dtos[0].key_mask, mask_key(key));
        assert_eq!(dtos[0].picker_name.as_deref(), Some("api-claude-opus-5-5"));
        assert_eq!(dtos[1].picker_name, None);
        assert!(s.contains("keyMask") && s.contains("pickerName") && s.contains("verifiedAt"));
        set_entries(vec![]);
    }

    #[test]
    fn load_save_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(dir.path()).is_empty());
        let v = vec![api("a", "claude-opus-5-5", "k1", Some(5)), api("b", "claude-haiku-4-5", "k2", None)];
        save(dir.path(), &v).unwrap();
        assert_eq!(load(dir.path()), v);
    }

    #[test]
    fn key_with_control_or_non_ascii_rejected() {
        assert!(validate_key("sk-ant-abc123").is_ok());
        for bad in ["", "sk ant", "sk-ant\n", "sk-ant\t1", "密钥abc", "sk-é", "a\u{7f}b"] {
            assert!(validate_key(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn failed_reverify_keeps_old_entry() {
        let existing = vec![api("a", "claude-opus-5-5", "old", Some(9))];
        let cand = api("a", "claude-opus-5-5", "new", None);
        assert_eq!(merge_edit(&existing, cand.clone(), false), existing);
        let ok = merge_edit(&existing, api("a", "claude-opus-5-5", "new", Some(10)), true);
        assert_eq!(ok[0].api_key, "new");
        // 新增失败：照样保存为未通过条目
        let added = merge_edit(&existing, api("b", "claude-haiku-4-5", "k", None), false);
        assert_eq!(added.len(), 2);
        assert_eq!(added[1].verified_at, None);
    }
}
