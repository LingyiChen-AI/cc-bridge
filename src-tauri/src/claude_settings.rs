//! Claude Code `settings.json` 的接管 / 撤销。只动本客户端写入的内容。
use crate::config::{TakeoverRecord, write_atomic};
use serde_json::{Map, Value, json};
use std::{fs, io, path::Path};

const BASE_URL_KEY: &str = "ANTHROPIC_BASE_URL";
const ENV_MODEL_KEY: &str = "ANTHROPIC_MODEL";

#[derive(Debug)]
pub enum SettingsError {
    Io(io::Error),
    Invalid(String),
}

impl std::fmt::Display for SettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Invalid(e) => write!(f, "invalid: {e}"),
        }
    }
}

pub fn read_settings(path: &Path) -> Result<Value, SettingsError> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(json!({})),
        Err(e) => return Err(SettingsError::Io(e)),
    };
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(json!({}));
    }
    match serde_json::from_slice::<Value>(bytes) {
        Ok(v @ Value::Object(_)) => Ok(v),
        Ok(_) => Err(SettingsError::Invalid("top-level value is not an object".into())),
        Err(e) => Err(SettingsError::Invalid(e.to_string())),
    }
}

pub fn write_settings(path: &Path, v: &Value) -> io::Result<()> {
    let mut s = serde_json::to_string_pretty(v).map_err(io::Error::other)?;
    s.push('\n');
    write_atomic(path, s.as_bytes())
}

fn obj(v: &mut Value) -> &mut Map<String, Value> {
    if !v.is_object() {
        *v = json!({});
    }
    v.as_object_mut().unwrap()
}

fn child<'a>(parent: &'a mut Map<String, Value>, key: &str) -> &'a mut Map<String, Value> {
    obj(parent.entry(key.to_string()).or_insert_with(|| json!({})))
}

fn str_at<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter().try_fold(v, |cur, k| cur.get(k)).and_then(Value::as_str)
}

pub fn model_entry(id: &str) -> Value {
    if let Some(rest) = id.strip_prefix("api-") {
        return match crate::proxy::custom_api::lookup(id) {
            Some(e) => {
                let host = e.base_url.split("://").nth(1).unwrap_or(&e.base_url).split('/').next().unwrap_or("");
                json!({ "model": id, "label": format!("API {}", e.model), "description": format!("自己的 API · {host}") })
            }
            None => json!({ "model": id, "label": format!("API {rest}"), "description": "自己的 API" }),
        };
    }
    json!({ "model": id, "label": format!("Codex {id}"), "description": "ChatGPT subscription" })
}

pub fn is_taken_over(doc: &Value, rec: &TakeoverRecord) -> bool {
    rec.base_url.is_some() && str_at(doc, &["env", BASE_URL_KEY]) == rec.base_url.as_deref()
}

/// 有本客户端写入的残留（崩溃恢复用）。
pub fn has_leftover(doc: &Value, rec: &TakeoverRecord) -> bool {
    is_taken_over(doc, rec) || !rec.models.is_empty()
}

pub fn apply_takeover(doc: &mut Value, rec: &mut TakeoverRecord, base_url: &str, models: &[String]) {
    let root = obj(doc);
    if rec.base_url.is_none() {
        rec.created_env = !root.contains_key("env");
        rec.created_picker = !root.contains_key("modelPicker");
    }
    if rec.prev_model.is_none() {
        rec.prev_model = Some(root.get("model").and_then(Value::as_str).map(String::from));
    }
    let env = child(root, "env");
    if rec.prev_env_model.is_none() {
        rec.prev_env_model = Some(env.get(ENV_MODEL_KEY).and_then(Value::as_str).map(String::from));
    }
    env.insert(BASE_URL_KEY.into(), Value::String(base_url.into()));
    rec.base_url = Some(base_url.into());

    let picker = child(root, "modelPicker");
    let options = picker.entry("options").or_insert_with(|| json!([]));
    if !options.is_array() {
        *options = json!([]);
    }
    let arr = options.as_array_mut().unwrap();
    arr.retain(|o| !o.get("model").and_then(Value::as_str).is_some_and(|m| rec.models.iter().any(|x| x == m)));
    arr.extend(models.iter().map(|m| model_entry(m)));
    rec.models = models.to_vec();
}

/// Claude Code 自己的模型名：别名（opus / sonnet / haiku / fable / opusplan / default …）或 claude-* 完整 ID，可带 [1m] 之类后缀。
pub fn is_claude_model(m: &str) -> bool {
    let m = m.trim().to_ascii_lowercase();
    let base = m.split('[').next().unwrap_or("").trim();
    base.is_empty()
        || base.starts_with("claude")
        || ["default", "best", "opusplan"].contains(&base)
        || ["opus", "sonnet", "haiku", "fable"].iter().any(|f| base == *f || base.starts_with(&format!("{f}-")))
        || base.contains("anthropic")
}

pub fn revoke_takeover(doc: &mut Value, rec: &mut TakeoverRecord) -> Vec<String> {
    let mut warnings = Vec::new();
    let root = obj(doc);
    // 接管期间 base_url 指向本代理，此时 model 里任何非 Claude 模型都只能是经由本代理选的 Codex 模型
    // （包括以前勾选过、现在已取消勾选的，或在 /model 里手动输入的），一律恢复。
    // `api-` 开头的是本代理的自定义 API 条目，模型名可以是任意兼容模型（甚至含 anthropic 字样），同样恢复。
    let ours = |m: &str| rec.models.iter().any(|x| x == m) || m.starts_with("api-") || !is_claude_model(m);

    if root.get("model").and_then(Value::as_str).is_some_and(ours) {
        match rec.prev_model.clone().flatten() {
            Some(prev) => {
                root.insert("model".into(), Value::String(prev));
            }
            None => {
                root.shift_remove("model");
            }
        }
    }

    if let Some(env) = root.get_mut("env").and_then(Value::as_object_mut) {
        match (env.get(BASE_URL_KEY).and_then(Value::as_str), rec.base_url.as_deref()) {
            (Some(cur), Some(mine)) if cur == mine => {
                env.shift_remove(BASE_URL_KEY);
            }
            (Some(cur), _) => warnings.push(format!("ANTHROPIC_BASE_URL 已被改成 {cur}，未改动")),
            _ => {}
        }
        if env.get(ENV_MODEL_KEY).and_then(Value::as_str).is_some_and(ours) {
            match rec.prev_env_model.clone().flatten() {
                Some(prev) => {
                    env.insert(ENV_MODEL_KEY.into(), Value::String(prev));
                }
                None => {
                    env.shift_remove(ENV_MODEL_KEY);
                }
            }
        }
        if env.is_empty() && rec.created_env {
            root.shift_remove("env");
        }
    }

    if let Some(picker) = root.get_mut("modelPicker").and_then(Value::as_object_mut) {
        if let Some(arr) = picker.get_mut("options").and_then(Value::as_array_mut) {
            // 列表里只删本客户端写入的条目，用户自己加的保留
            let written = |m: &str| rec.models.iter().any(|x| x == m);
            arr.retain(|o| !o.get("model").and_then(Value::as_str).is_some_and(written));
            if arr.is_empty() && rec.created_picker {
                picker.shift_remove("options");
            }
        }
        if picker.is_empty() && rec.created_picker {
            root.shift_remove("modelPicker");
        }
    }

    *rec = TakeoverRecord::default();
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn models(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }
    const URL: &str = "http://127.0.0.1:18765";

    #[test]
    fn takeover_roundtrip_preserves_user_keys() {
        let original = json!({"env":{"HTTPS_PROXY":"x"},"hooks":{"a":1},"modelPicker":{"options":[{"model":"mine","label":"Mine"}]},"model":"opus"});
        let mut doc = original.clone();
        let mut rec = TakeoverRecord::default();
        apply_takeover(&mut doc, &mut rec, URL, &models(&["gpt-a", "gpt-b"]));
        let opts: Vec<&str> = doc["modelPicker"]["options"].as_array().unwrap().iter().map(|o| o["model"].as_str().unwrap()).collect();
        assert_eq!(opts, ["mine", "gpt-a", "gpt-b"]);
        assert_eq!(doc["modelPicker"]["options"][1]["label"], "Codex gpt-a");
        assert_eq!(doc["env"]["ANTHROPIC_BASE_URL"], URL);
        assert!(is_taken_over(&doc, &rec));
        doc["model"] = json!("gpt-a");
        let w = revoke_takeover(&mut doc, &mut rec);
        assert!(w.is_empty());
        assert_eq!(serde_json::to_string(&doc).unwrap(), serde_json::to_string(&original).unwrap());
    }

    #[test]
    fn revoke_restores_model_not_in_current_selection() {
        // 复现：选过 gpt-6.1-sol 后又取消勾选，撤销时顶层 model 残留，CC 新窗口不可用
        let mut doc = json!({"model":"opus","env":{"HTTPS_PROXY":"x"}});
        let mut rec = TakeoverRecord::default();
        apply_takeover(&mut doc, &mut rec, URL, &models(&["gpt-6.2-sol", "gpt-6-sol"]));
        doc["model"] = json!("gpt-6.1-sol");
        doc["env"]["ANTHROPIC_MODEL"] = json!("gpt-5-codex");
        revoke_takeover(&mut doc, &mut rec);
        assert_eq!(doc, json!({"model":"opus","env":{"HTTPS_PROXY":"x"}}));
    }

    #[test]
    fn revoke_keeps_user_claude_model_choice() {
        let mut doc = json!({"model":"opus"});
        let mut rec = TakeoverRecord::default();
        apply_takeover(&mut doc, &mut rec, URL, &models(&["gpt-a"]));
        for m in ["sonnet", "sonnet[1m]", "claude-opus-5-5", "haiku", "fable"] {
            let mut d = doc.clone();
            let mut r = rec.clone();
            d["model"] = json!(m);
            revoke_takeover(&mut d, &mut r);
            assert_eq!(d["model"], m);
        }
    }

    #[test]
    fn claude_model_detection() {
        for m in ["opus", "Sonnet", "opus[1m]", "opusplan", "default", "claude-sonnet-5-5", "haiku-5-5", "fable"] {
            assert!(is_claude_model(m), "{m}");
        }
        for m in ["gpt-6.1-sol", "gpt-5-codex", "o3", "codex-mini"] {
            assert!(!is_claude_model(m), "{m}");
        }
    }

    #[test]
    fn apply_twice_replaces_models() {
        let mut doc = json!({"modelPicker":{"options":[{"model":"mine"}]}});
        let mut rec = TakeoverRecord::default();
        apply_takeover(&mut doc, &mut rec, URL, &models(&["gpt-a"]));
        apply_takeover(&mut doc, &mut rec, URL, &models(&["gpt-c"]));
        let opts: Vec<&str> = doc["modelPicker"]["options"].as_array().unwrap().iter().map(|o| o["model"].as_str().unwrap()).collect();
        assert_eq!(opts, ["mine", "gpt-c"]);
    }

    #[test]
    fn revoke_keeps_user_modified_base_url() {
        let mut doc = json!({});
        let mut rec = TakeoverRecord::default();
        apply_takeover(&mut doc, &mut rec, URL, &models(&["gpt-a"]));
        doc["env"]["ANTHROPIC_BASE_URL"] = json!("http://other");
        let w = revoke_takeover(&mut doc, &mut rec);
        assert_eq!(doc["env"]["ANTHROPIC_BASE_URL"], "http://other");
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn revoke_on_created_file_leaves_empty_object() {
        let mut doc = json!({});
        let mut rec = TakeoverRecord::default();
        apply_takeover(&mut doc, &mut rec, URL, &models(&["gpt-a"]));
        doc["model"] = json!("gpt-a");
        revoke_takeover(&mut doc, &mut rec);
        assert_eq!(doc, json!({}));
    }

    #[test]
    fn read_settings_bom_ok_and_invalid_err() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("settings.json");
        assert_eq!(read_settings(&p).unwrap(), json!({}));
        fs::write(&p, b"\xEF\xBB\xBF{\"a\":1}").unwrap();
        assert_eq!(read_settings(&p).unwrap(), json!({"a":1}));
        fs::write(&p, b"{\"a\":1,}").unwrap();
        assert!(matches!(read_settings(&p), Err(SettingsError::Invalid(_))));
        fs::write(&p, b"// c\n{}").unwrap();
        assert!(matches!(read_settings(&p), Err(SettingsError::Invalid(_))));
    }

    #[test]
    fn write_creates_missing_dir() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("nested").join(".claude").join("settings.json");
        write_settings(&p, &json!({"a":1})).unwrap();
        assert_eq!(read_settings(&p).unwrap(), json!({"a":1}));
    }

    #[test]
    fn api_entry_label_and_description() {
        use crate::proxy::custom_api::{AuthStyle, CustomApi, TEST_LOCK, set_entries};
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_entries(vec![CustomApi {
            id: "x".into(),
            base_url: "https://relay.example.com".into(),
            api_key: "k".into(),
            model: "claude-opus-5-5".into(),
            auth_style: AuthStyle::XApiKey,
            verified_at: Some(1),
        }]);
        assert_eq!(
            model_entry("api-claude-opus-5-5"),
            json!({"model":"api-claude-opus-5-5","label":"API claude-opus-5-5","description":"自己的 API · relay.example.com"})
        );
        set_entries(vec![]);
        assert_eq!(
            model_entry("api-claude-opus-5-5"),
            json!({"model":"api-claude-opus-5-5","label":"API claude-opus-5-5","description":"自己的 API"})
        );
        assert_eq!(model_entry("gpt-a")["label"], "Codex gpt-a");
    }

    #[test]
    fn revoke_restores_model_when_api_entry_selected() {
        let mut doc = json!({"model":"opus"});
        let mut rec = TakeoverRecord::default();
        apply_takeover(&mut doc, &mut rec, URL, &models(&["gpt-6.2-sol", "api-claude-opus-5-5"]));
        doc["model"] = json!("api-claude-opus-5-5");
        revoke_takeover(&mut doc, &mut rec);
        assert_eq!(doc["model"], "opus");
        assert!(doc.get("modelPicker").is_none());
    }

    #[test]
    fn non_claude_api_models_roundtrip_through_settings() {
        use crate::proxy::custom_api::{AuthStyle, CustomApi, TEST_LOCK, picker_list, set_entries};
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let e = |id: &str, model: &str| CustomApi {
            id: id.into(),
            base_url: "https://dashscope.example.com/apps/anthropic".into(),
            api_key: "k".into(),
            model: model.into(),
            auth_style: AuthStyle::Bearer,
            verified_at: Some(1),
        };
        set_entries(vec![e("1", "Qwen/Qwen3-Coder"), e("2", "kimi-k2:latest"), e("3", "anthropic/claude-x")]);
        let names = picker_list();
        let mut doc = json!({"model":"opus"});
        let mut rec = TakeoverRecord::default();
        apply_takeover(&mut doc, &mut rec, URL, &names);
        let opts = doc["modelPicker"]["options"].as_array().unwrap();
        assert_eq!(
            opts[0],
            json!({"model":"api-Qwen/Qwen3-Coder","label":"API Qwen/Qwen3-Coder","description":"自己的 API · dashscope.example.com"})
        );
        assert_eq!(opts[1]["model"], "api-kimi-k2:latest");
        // 写进 settings.json 再读回，模型名原样保留（`/`、`:`、大写都不转义、不改写）。
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("settings.json");
        write_settings(&p, &doc).unwrap();
        let back = read_settings(&p).unwrap();
        assert_eq!(back["modelPicker"]["options"][0]["model"], "api-Qwen/Qwen3-Coder");
        // 选中过的自定义条目（含名字里带 anthropic 的）撤销时都恢复成原模型。
        for m in ["api-Qwen/Qwen3-Coder", "api-anthropic/claude-x"] {
            let (mut d, mut r) = (doc.clone(), rec.clone());
            d["model"] = json!(m);
            r.models.retain(|x| x != m);
            revoke_takeover(&mut d, &mut r);
            assert_eq!(d["model"], "opus", "{m}");
        }
        set_entries(vec![]);
    }
}
