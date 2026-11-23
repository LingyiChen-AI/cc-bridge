//! 自定义 Claude API 转发：把 `api-<model>` 的请求改写后发往用户自己的 base URL，换上用户的 key。
//! 订阅透传（`providers::anthropic`）不受影响；key 只出现在出站请求头里，不进日志 / 统计 / 错误信息。

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::Response;
use serde_json::Value;

use super::{AuthStyle, CustomApi, lookup};
use crate::proxy::anthropic::error::json_error;
use crate::proxy::anthropic::schema::MessagesRequest;
use crate::proxy::provider::{CliHandlers, Provider, RequestContext};
use crate::proxy::providers::anthropic::{
    is_stripped_request_header, is_stripped_response_header, sanitize_anthropic_request,
};

/// 与 `AnthropicProvider::new` 相同：走客户端配置的对外代理，禁止重定向。
pub(crate) fn build_client() -> reqwest::Client {
    crate::proxy::providers::codex::client::apply_outbound_proxy(
        reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()),
    )
    .build()
    .expect("failed to build custom api client")
}

/// 把顶层 `model` 改成真实模型名，并去掉中转站不支持的 `thread`（Message Threads）；
/// 非 JSON 返回 None（原样转发）。
pub(crate) fn rewrite_model(raw: &[u8], real_model: &str) -> Option<Vec<u8>> {
    let mut doc: Value = serde_json::from_slice(raw).ok()?;
    let obj = doc.as_object_mut()?;
    obj.insert("model".into(), Value::String(real_model.to_string()));
    obj.remove("thread");
    serde_json::to_vec(&doc).ok()
}

/// 选条目用的模型名：优先解析后的 `body.model`（已应用 auto-review 等覆盖，路由也按它选的），
/// 缺失时退回原始请求体里的 `model`。
pub(crate) fn entry_model(body_model: Option<String>, raw: &[u8]) -> Option<String> {
    body_model.or_else(|| {
        serde_json::from_slice::<Value>(raw)
            .ok()
            .and_then(|v| v.get("model").and_then(Value::as_str).map(str::to_string))
    })
}

/// 去掉入站凭证、订阅专用的 `oauth-*` beta 和中转站不支持的 `message-threads-*` beta，换上用户的 key。
pub(crate) fn outbound_headers(incoming: &HeaderMap, entry: &CustomApi) -> HeaderMap {
    let mut out = HeaderMap::with_capacity(incoming.len() + 1);
    let mut betas: Vec<String> = Vec::new();
    for (name, value) in incoming.iter() {
        let n = name.as_str();
        if is_stripped_request_header(n) || n == "authorization" || n == "x-api-key" {
            continue;
        }
        if n == "anthropic-beta" {
            if let Ok(v) = value.to_str() {
                betas.extend(
                    v.split(',')
                        .map(str::trim)
                        .filter(|f| {
                            !f.is_empty()
                                && !f.starts_with("oauth-")
                                && !f.starts_with("message-threads-")
                        })
                        .map(str::to_string),
                );
            }
            continue;
        }
        out.append(name.clone(), value.clone());
    }
    if !betas.is_empty()
        && let Ok(v) = HeaderValue::from_str(&betas.join(","))
    {
        out.insert(HeaderName::from_static("anthropic-beta"), v);
    }
    match entry.auth_style {
        AuthStyle::XApiKey => {
            if let Ok(mut v) = HeaderValue::from_str(&entry.api_key) {
                v.set_sensitive(true);
                out.insert(HeaderName::from_static("x-api-key"), v);
            }
        }
        AuthStyle::Bearer => {
            if let Ok(mut v) = HeaderValue::from_str(&format!("Bearer {}", entry.api_key)) {
                v.set_sensitive(true);
                out.insert(HeaderName::from_static("authorization"), v);
            }
        }
    }
    out
}

pub struct CustomApiProvider {
    client: reqwest::Client,
}

impl CustomApiProvider {
    pub fn new() -> Self {
        Self { client: build_client() }
    }

    async fn relay(&self, body_model: Option<String>, ctx: RequestContext) -> Response {
        let RequestContext {
            req_id,
            monitor,
            passthrough,
            ..
        } = ctx;
        let Some(passthrough) = passthrough else {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "api_error",
                "custom api relay is missing the original request",
            );
        };

        let Some(entry) = entry_model(body_model, &passthrough.raw_body).as_deref().and_then(lookup) else {
            return json_error(
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                "自定义 API 已被删除或未通过校验",
            );
        };

        let url = format!("{}{}", entry.base_url, passthrough.path_and_query);
        let headers = outbound_headers(&passthrough.headers, &entry);

        if let Some(monitor) = monitor.as_ref() {
            monitor.upstream_started(&req_id);
        }

        let rewritten = rewrite_model(&passthrough.raw_body, &entry.model);
        let base: &[u8] = rewritten.as_deref().unwrap_or(&passthrough.raw_body);
        let outgoing = match sanitize_anthropic_request(base, &req_id) {
            Some(bytes) => reqwest::Body::from(bytes),
            None => reqwest::Body::from(base.to_vec()),
        };

        match self
            .client
            .post(&url)
            .headers(headers)
            .body(outgoing)
            .send()
            .await
        {
            Ok(upstream) => {
                let status = upstream.status();
                let mut out_headers = HeaderMap::with_capacity(upstream.headers().len());
                for (name, value) in upstream.headers() {
                    if is_stripped_response_header(name.as_str()) {
                        continue;
                    }
                    out_headers.append(name.clone(), value.clone());
                }
                let mut response = Response::new(Body::from_stream(upstream.bytes_stream()));
                *response.status_mut() = status;
                *response.headers_mut() = out_headers;
                response
            }
            Err(err) => json_error(
                StatusCode::BAD_GATEWAY,
                "api_error",
                format!("custom api upstream request failed: {}", err.without_url()),
            ),
        }
    }
}

impl Default for CustomApiProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Provider for CustomApiProvider {
    fn name(&self) -> &'static str {
        "custom-api"
    }

    fn supported_models(&self) -> Vec<String> {
        super::picker_list()
    }

    fn cli(&self) -> &'static dyn CliHandlers {
        &CUSTOM_API_CLI
    }

    async fn handle_messages(&self, body: MessagesRequest, ctx: RequestContext) -> Response {
        self.relay(body.model, ctx).await
    }

    async fn handle_count_tokens(&self, body: MessagesRequest, ctx: RequestContext) -> Response {
        self.relay(body.model, ctx).await
    }
}

pub struct CustomApiCli;
pub static CUSTOM_API_CLI: CustomApiCli = CustomApiCli;

impl CliHandlers for CustomApiCli {
    fn login(&self) -> anyhow::Result<()> {
        anyhow::bail!("自定义 API 在应用设置里配置，无需登录")
    }
    fn device(&self) -> anyhow::Result<()> {
        anyhow::bail!("自定义 API 在应用设置里配置，无需登录")
    }
    fn status(&self) -> anyhow::Result<()> {
        println!("Custom API backend: relays to user-configured base URLs");
        Ok(())
    }
    fn logout(&self) -> anyhow::Result<()> {
        println!("Custom API backend stores no login; nothing to remove");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::config::AliasProvider;
    use crate::proxy::custom_api::{AuthStyle, TEST_LOCK, set_entries};
    use crate::proxy::provider::Passthrough;
    use axum::http::HeaderValue;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn entry(base_url: &str, style: AuthStyle) -> CustomApi {
        CustomApi {
            id: "1".into(),
            base_url: base_url.into(),
            api_key: "sk-user-KEY".into(),
            model: "claude-opus-5-5".into(),
            auth_style: style,
            verified_at: Some(1),
        }
    }

    fn incoming() -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("authorization", HeaderValue::from_static("Bearer sk-ant-oat01-SUB"));
        h.insert("x-api-key", HeaderValue::from_static("x"));
        h.insert(
            "anthropic-beta",
            HeaderValue::from_static("oauth-2025-04-20,interleaved-thinking-2025-05-14"),
        );
        h.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
        h
    }

    #[test]
    fn rewrite_model_only_touches_model() {
        let raw = br#"{"model":"api-claude-opus-5-5","max_tokens":10,"messages":[{"role":"user","content":"hi"}]}"#;
        let out: Value =
            serde_json::from_slice(&rewrite_model(raw, "claude-opus-5-5").unwrap()).unwrap();
        assert_eq!(out["model"], "claude-opus-5-5");
        assert_eq!(out["messages"][0]["content"], "hi");
        assert!(rewrite_model(b"not json", "claude-opus-5-5").is_none());
    }

    #[test]
    fn rewrite_model_drops_thread() {
        let raw = br#"{"model":"api-x","messages":[{"role":"user","content":"hi"}],"thread":{"type":"create"}}"#;
        let out: Value = serde_json::from_slice(&rewrite_model(raw, "x").unwrap()).unwrap();
        assert!(out.get("thread").is_none());
        assert_eq!(out["messages"][0]["content"], "hi");
    }

    #[test]
    fn outbound_headers_drop_message_threads_beta() {
        let mut h = incoming();
        h.insert(
            "anthropic-beta",
            HeaderValue::from_static("message-threads-2026-08-12,interleaved-thinking-2025-05-14"),
        );
        let out = outbound_headers(&h, &entry("https://a", AuthStyle::XApiKey));
        assert_eq!(out.get("anthropic-beta").unwrap(), "interleaved-thinking-2025-05-14");
    }

    #[test]
    fn entry_model_prefers_parsed_body() {
        let raw = br#"{"model":"api-claude-opus-5-5"}"#;
        assert_eq!(
            entry_model(Some("api-claude-sonnet-5-5".into()), raw).as_deref(),
            Some("api-claude-sonnet-5-5")
        );
        assert_eq!(entry_model(None, raw).as_deref(), Some("api-claude-opus-5-5"));
        assert_eq!(entry_model(None, b"not json"), None);
    }

    #[test]
    fn outbound_headers_swap_credentials() {
        let h = outbound_headers(&incoming(), &entry("https://a", AuthStyle::XApiKey));
        assert!(h.get("authorization").is_none());
        assert_eq!(h.get("x-api-key").unwrap(), "sk-user-KEY");
        assert_eq!(h.get("anthropic-beta").unwrap(), "interleaved-thinking-2025-05-14");
        assert_eq!(h.get("anthropic-version").unwrap(), "2023-06-01");

        let h = outbound_headers(&incoming(), &entry("https://a", AuthStyle::Bearer));
        assert_eq!(h.get("authorization").unwrap(), "Bearer sk-user-KEY");
        assert!(h.get("x-api-key").is_none());

        let mut only_oauth = incoming();
        only_oauth.insert("anthropic-beta", HeaderValue::from_static("oauth-2025-04-20"));
        let h = outbound_headers(&only_oauth, &entry("https://a", AuthStyle::XApiKey));
        assert!(h.get("anthropic-beta").is_none());
    }

    #[test]
    fn debug_never_prints_raw_key() {
        let e = entry("https://a", AuthStyle::XApiKey);
        let dbg = format!("{:?}", e);
        assert!(!dbg.contains("sk-user-KEY"), "{dbg}");
    }

    async fn fake_upstream(
        sse: &'static [u8],
    ) -> (String, tokio::task::JoinHandle<(String, String, Value)>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut req = Vec::new();
            let mut buf = [0_u8; 4096];
            let header_end = loop {
                let n = socket.read(&mut buf).await.unwrap();
                assert!(n > 0);
                req.extend_from_slice(&buf[..n]);
                if let Some(p) = req.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&req[..p]).to_ascii_lowercase();
                    let len = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:")?.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    if req.len() >= p + 4 + len {
                        break p;
                    }
                }
            };
            let head = String::from_utf8_lossy(&req[..header_end]).to_string();
            let path = head.lines().next().unwrap().split(' ').nth(1).unwrap().to_string();
            let body: Value = serde_json::from_slice(&req[header_end + 4..]).unwrap();
            let resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                sse.len()
            );
            socket.write_all(resp.as_bytes()).await.unwrap();
            socket.write_all(sse).await.unwrap();
            (path, head, body)
        });
        (format!("http://{addr}"), task)
    }

    fn ctx_for(raw: &'static [u8], headers: HeaderMap, path: &str) -> RequestContext {
        RequestContext {
            req_id: "t".into(),
            session_id: None,
            session_seq: None,
            provider: "custom-api".into(),
            traffic: None,
            monitor: None,
            passthrough: Some(Passthrough {
                raw_body: axum::body::Bytes::from_static(raw),
                headers,
                path_and_query: path.into(),
            }),
        }
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn relays_to_entry_base_url_with_user_key() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let sse: &'static [u8] = b"event: message_start\ndata: {}\n\n";
        let (base, task) = fake_upstream(sse).await;
        set_entries(vec![entry(&base, AuthStyle::XApiKey)]);

        let raw: &'static [u8] = br#"{"model":"api-claude-opus-5-5[1m]","max_tokens":10,"messages":[{"role":"user","content":"hi"}]}"#;
        let ctx = ctx_for(raw, incoming(), "/v1/messages?beta=true");
        let body: MessagesRequest = serde_json::from_slice(raw).unwrap();
        let resp = CustomApiProvider::new().handle_messages(body, ctx).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&bytes[..], sse);

        let (path, head, body) = task.await.unwrap();
        assert_eq!(path, "/v1/messages?beta=true");
        assert_eq!(body["model"], "claude-opus-5-5");
        let head = head.to_ascii_lowercase();
        assert!(head.contains("x-api-key: sk-user-key"));
        assert!(!head.contains("sk-ant-oat01-sub"));
        set_entries(vec![]);
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn deleted_entry_returns_400() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_entries(vec![]);
        let raw: &'static [u8] = br#"{"model":"api-claude-opus-5-5","messages":[]}"#;
        let ctx = ctx_for(raw, HeaderMap::new(), "/v1/messages");
        let body: MessagesRequest = serde_json::from_slice(raw).unwrap();
        let resp = CustomApiProvider::new().handle_messages(body, ctx).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn routes_and_rewrites_non_claude_model_names() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let sse: &'static [u8] = b"event: message_start\ndata: {}\n\n";
        let (base, task) = fake_upstream(sse).await;
        let mut e = entry(&base, AuthStyle::Bearer);
        e.model = "Qwen/Qwen3-Coder:latest".into();
        set_entries(vec![e]);
        let reg = crate::proxy::registry::Registry::new(AliasProvider::Anthropic);
        for name in ["api-Qwen/Qwen3-Coder:latest", "api-Qwen/Qwen3-Coder:latest[1m]", "api-qwen/qwen3-coder:latest"] {
            assert_eq!(reg.provider_for_model(name, None).unwrap().name(), "custom-api", "{name}");
        }

        let raw: &'static [u8] = br#"{"model":"api-Qwen/Qwen3-Coder:latest[1m]","max_tokens":10,"messages":[{"role":"user","content":"hi"}]}"#;
        let ctx = ctx_for(raw, incoming(), "/v1/messages");
        let body: MessagesRequest = serde_json::from_slice(raw).unwrap();
        let resp = CustomApiProvider::new().handle_messages(body, ctx).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let (_, head, body) = task.await.unwrap();
        assert_eq!(body["model"], "Qwen/Qwen3-Coder:latest");
        assert!(head.to_ascii_lowercase().contains("authorization: bearer sk-user-key"));
        set_entries(vec![]);
    }

    #[test]
    fn routes_api_name_with_1m_suffix() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let verified = entry("https://a", AuthStyle::XApiKey);
        let mut unverified = entry("https://a", AuthStyle::XApiKey);
        unverified.model = "claude-sonnet-5-5".into();
        unverified.verified_at = None;
        set_entries(vec![verified, unverified]);
        let reg = crate::proxy::registry::Registry::new(AliasProvider::Anthropic);
        assert_eq!(
            reg.provider_for_model("api-claude-opus-5-5[1m]", None).unwrap().name(),
            "custom-api"
        );
        assert_eq!(
            reg.provider_for_model("claude-opus-5-5", None).unwrap().name(),
            "anthropic"
        );
        assert!(reg.provider_for_model("api-claude-sonnet-5-5", None).is_none());
        set_entries(vec![]);
    }
}
