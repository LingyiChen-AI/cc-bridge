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
            Self::Io(e) => write!(f, "读取失败：{e}"),
            Self::Invalid(e) => write!(f, "settings.json 不是合法 JSON，已停止修改：{e}"),
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
        Ok(_) => Err(SettingsError::Invalid("顶层不是对象".into())),
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

pub fn revoke_takeover(doc: &mut Value, rec: &mut TakeoverRecord) -> Vec<String> {
    let mut warnings = Vec::new();
    let root = obj(doc);
    let ours = |m: &str| rec.models.iter().any(|x| x == m);

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
            arr.retain(|o| !o.get("model").and_then(Value::as_str).is_some_and(ours));
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
}
