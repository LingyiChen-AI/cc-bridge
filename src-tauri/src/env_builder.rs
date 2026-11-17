//! 构造代理子进程的环境变量：不继承会干扰代理的变量，并补齐 HOME。
use crate::config::AppConfig;
use std::{collections::HashMap, path::Path};

const PROXY_VARS: [&str; 4] = ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY"];
pub const NO_PROXY_VALUE: &str = "localhost,127.0.0.1,::1";

fn should_drop(key: &str) -> bool {
    let k = key.to_ascii_uppercase();
    k.starts_with("CODEX") || k.starts_with("CCP_") || k.starts_with("ANTHROPIC_") || k == "HOME" || PROXY_VARS.contains(&k.as_str())
}

pub fn build_proxy_env(base: &HashMap<String, String>, home: &Path, cfg: &AppConfig) -> HashMap<String, String> {
    let mut env: HashMap<String, String> = base.iter().filter(|(k, _)| !should_drop(k)).map(|(k, v)| (k.clone(), v.clone())).collect();
    env.insert("HOME".into(), home.to_string_lossy().into_owned());
    env.insert("CCP_LOG_STDERR".into(), "1".into());
    if let Some(dir) = &cfg.codex_home {
        env.insert("CODEX_HOME".into(), dir.to_string_lossy().into_owned());
    }
    if let Some(proxy) = cfg.outbound() {
        env.insert("HTTP_PROXY".into(), proxy.into());
        env.insert("HTTPS_PROXY".into(), proxy.into());
        env.insert("NO_PROXY".into(), NO_PROXY_VALUE.into());
    }
    env
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn base() -> HashMap<String, String> {
        [("CODEX_CI", "1"), ("codex_home", "x"), ("CCP_PORT", "1"), ("ANTHROPIC_BASE_URL", "u"), ("https_proxy", "p"), ("PATH", "a")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn env_strips_and_sets() {
        let cfg = AppConfig { outbound_proxy: Some("http://127.0.0.1:7890".into()), ..Default::default() };
        let home = PathBuf::from("C:\\Users\\张 三");
        let env = build_proxy_env(&base(), &home, &cfg);
        for k in ["CODEX_CI", "codex_home", "CCP_PORT", "ANTHROPIC_BASE_URL", "https_proxy", "CODEX_HOME"] {
            assert!(!env.contains_key(k), "{k} 应被移除");
        }
        assert_eq!(env["PATH"], "a");
        assert_eq!(env["HOME"], "C:\\Users\\张 三");
        assert_eq!(env["HTTP_PROXY"], "http://127.0.0.1:7890");
        assert_eq!(env["HTTPS_PROXY"], "http://127.0.0.1:7890");
        assert_eq!(env["NO_PROXY"], NO_PROXY_VALUE);
        assert_eq!(env["CCP_LOG_STDERR"], "1");
    }

    #[test]
    fn env_no_outbound() {
        let cfg = AppConfig { outbound_proxy: Some("  ".into()), ..Default::default() };
        let env = build_proxy_env(&base(), Path::new("/home/u"), &cfg);
        for k in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY"] {
            assert!(!env.contains_key(k));
        }
    }
}
