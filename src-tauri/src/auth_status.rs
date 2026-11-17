//! 只读检测 Codex CLI 的登录文件（不修改）。
use base64::Engine;
use serde::Serialize;
use serde_json::Value;
use std::{collections::HashMap, fs, path::{Path, PathBuf}};

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum AuthState {
    /// 令牌有效，`expires_at` 为 unix 秒
    Ok { expires_at: i64 },
    /// 已过期但有 refresh_token，代理下次请求会自动续期
    ExpiredRefreshable,
    /// code: notFound | invalidJson | noToken | expired（前端按语言翻译）
    Missing { code: String },
}

/// 与上游一致的定位顺序：设置项 > CODEX_HOME > ~/.codex
pub fn auth_file(cfg_codex_home: Option<&Path>, env: &HashMap<String, String>, home: &Path) -> PathBuf {
    if let Some(d) = cfg_codex_home {
        return d.join("auth.json");
    }
    if let Some(d) = env.get("CODEX_HOME").filter(|s| !s.is_empty()) {
        return PathBuf::from(d).join("auth.json");
    }
    home.join(".codex").join("auth.json")
}

pub fn jwt_exp(token: &str) -> Option<i64> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice::<Value>(&bytes).ok()?.get("exp")?.as_i64()
}

pub fn inspect_auth(path: &Path, now_unix: i64) -> AuthState {
    let missing = |c: &str| AuthState::Missing { code: c.to_string() };
    let Ok(bytes) = fs::read(path) else {
        return missing("notFound");
    };
    let Ok(doc) = serde_json::from_slice::<Value>(&bytes) else {
        return missing("invalidJson");
    };
    let tokens = doc.get("tokens");
    let access = tokens.and_then(|t| t.get("access_token")).and_then(Value::as_str).unwrap_or("");
    if access.is_empty() {
        return missing("noToken");
    }
    let has_refresh = tokens.and_then(|t| t.get("refresh_token")).and_then(Value::as_str).is_some_and(|s| !s.is_empty());
    match jwt_exp(access) {
        Some(exp) if exp > now_unix => AuthState::Ok { expires_at: exp },
        _ if has_refresh => AuthState::ExpiredRefreshable,
        _ => missing("expired"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt(exp: i64) -> String {
        let p = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!("{{\"exp\":{exp}}}"));
        format!("e30.{p}.sig")
    }

    #[test]
    fn auth_states() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("auth.json");
        let now = 1_800_000_000;
        fs::write(&p, format!(r#"{{"tokens":{{"access_token":"{}","refresh_token":"r"}}}}"#, jwt(now + 60))).unwrap();
        assert_eq!(inspect_auth(&p, now), AuthState::Ok { expires_at: now + 60 });
        fs::write(&p, format!(r#"{{"tokens":{{"access_token":"{}","refresh_token":"r"}}}}"#, jwt(now - 60))).unwrap();
        assert_eq!(inspect_auth(&p, now), AuthState::ExpiredRefreshable);
        fs::write(&p, r#"{"auth_mode":"chatgpt"}"#).unwrap();
        assert!(matches!(inspect_auth(&p, now), AuthState::Missing { .. }));
        assert!(matches!(inspect_auth(&dir.path().join("none.json"), now), AuthState::Missing { .. }));
    }

    #[test]
    fn auth_file_priority() {
        let home = Path::new("/h");
        let mut env = HashMap::new();
        assert_eq!(auth_file(None, &env, home), home.join(".codex").join("auth.json"));
        env.insert("CODEX_HOME".to_string(), "/c".to_string());
        assert_eq!(auth_file(None, &env, home), PathBuf::from("/c").join("auth.json"));
        assert_eq!(auth_file(Some(Path::new("/cfg")), &env, home), PathBuf::from("/cfg").join("auth.json"));
    }
}
