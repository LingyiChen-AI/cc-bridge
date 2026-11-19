//! Claude（Anthropic Messages）协议校验：认证 → 响应格式，遇到第一个 Fail 即停。
//! 只检查协议形状，不判断背后是不是真 Claude：官方、中转站以及 Qwen、DeepSeek、Kimi 等的
//! Claude 兼容接口都能通过（2026-11-19 放宽，原分词器 / 签名 / 模型名三步已移除）。
//! key 只出现在出站请求头里；所有返回的文案都经 `scrub` 把 key 换成掩码。

use reqwest::header::HeaderValue;
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;

use super::{AuthStyle, mask_key};

const TIMEOUT: Duration = Duration::from_secs(30);
const NOT_MESSAGES_FORMAT: &str = "不是 Claude（Anthropic Messages）格式的接口";
const NOT_MESSAGES_STREAM: &str = "流式响应不是 Claude（Anthropic Messages）的事件格式";

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum StepId {
    Auth,
    Format,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum StepResult {
    Pass,
    Skip { reason: String },
    Fail { reason: String },
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct VerifyReport {
    pub steps: Vec<(StepId, StepResult)>,
    pub auth_style: Option<AuthStyle>,
    pub ok: bool,
    /// 失败是网络类（请求失败 / 超时、HTTP 5xx、429），不是对接口格式的判定；重新校验时据此保留已验证状态。
    pub transient: bool,
}

/// 最后一次上游请求的结果是否属于网络类问题：0 表示请求本身失败（连接 / 超时 / 读响应），以及 429、5xx。
pub(crate) fn is_transient_status(status: u16) -> bool {
    status == 0 || status == 429 || status >= 500
}

fn fail(reason: impl Into<String>) -> StepResult {
    StepResult::Fail { reason: reason.into() }
}

/// 把文案里出现的 key 换成掩码（上游错误信息可能回显 key）。
pub(crate) fn scrub(text: &str, api_key: &str) -> String {
    if api_key.is_empty() {
        text.to_string()
    } else {
        text.replace(api_key, &mask_key(api_key))
    }
}

/// reqwest 错误不带 URL、不用 Debug，只拼 Display 与 source 链。
fn describe_error(err: reqwest::Error) -> String {
    let err = err.without_url();
    let mut out = err.to_string();
    let mut src = std::error::Error::source(&err);
    while let Some(s) = src {
        out.push('：');
        out.push_str(&s.to_string());
        src = s.source();
    }
    out
}

struct Resp {
    status: u16,
    text: String,
}

impl Resp {
    fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
    fn json(&self) -> Value {
        serde_json::from_str(&self.text).unwrap_or(Value::Null)
    }
    /// `error.message`，没有就用原文；先去掉 key 再取前 200 字（否则跨截断点的 key 会漏掉）。
    fn error_message(&self, api_key: &str) -> String {
        let v = self.json();
        let msg = v["error"]["message"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| self.text.clone());
        scrub(&msg, api_key).chars().take(200).collect()
    }
}

struct Upstream<'a> {
    client: &'a reqwest::Client,
    base: String,
    key: &'a str,
    /// 最后一次请求的 HTTP 状态，0 表示请求失败（见 `is_transient_status`）。
    last: AtomicU16,
}

impl Upstream<'_> {
    async fn post(&self, path: &str, style: &AuthStyle, body: &Value) -> Result<Resp, String> {
        let (name, raw) = match style {
            AuthStyle::XApiKey => ("x-api-key", self.key.to_string()),
            AuthStyle::Bearer => ("authorization", format!("Bearer {}", self.key)),
        };
        let mut auth = HeaderValue::from_str(&raw).map_err(|_| "API Key 含有非法字符".to_string())?;
        auth.set_sensitive(true);
        self.last.store(0, Ordering::SeqCst);
        let resp = self
            .client
            .post(format!("{}{}", self.base, path))
            .timeout(TIMEOUT)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .header(name, auth)
            .body(body.to_string())
            .send()
            .await
            .map_err(|e| format!("请求失败：{}", describe_error(e)))?;
        let status = resp.status().as_u16();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("读取响应失败：{}", describe_error(e)))?;
        self.last.store(status, Ordering::SeqCst);
        Ok(Resp { status, text })
    }
}

fn http_error(r: &Resp, api_key: &str) -> String {
    format!("接口返回 HTTP {}：{}", r.status, r.error_message(api_key))
}

fn hello_body(model: &str, stream: bool) -> Value {
    let mut body = json!({
        "model": model,
        "max_tokens": 16,
        "messages": [{"role": "user", "content": "Reply with OK."}],
    });
    if stream {
        body["stream"] = json!(true);
    }
    body
}

/// 先用 x-api-key，401 / 403 再试 Bearer；返回探测到的认证方式与非流式响应体。
async fn step_auth(up: &Upstream<'_>, model: &str) -> (StepResult, Option<AuthStyle>, Value) {
    let body = hello_body(model, false);
    let mut last = None;
    for style in [AuthStyle::XApiKey, AuthStyle::Bearer] {
        let r = match up.post("/v1/messages", &style, &body).await {
            Ok(r) => r,
            Err(e) => return (fail(e), None, Value::Null),
        };
        if r.ok() {
            let v = r.json();
            return (StepResult::Pass, Some(style), v);
        }
        if r.status != 401 && r.status != 403 {
            return (fail(http_error(&r, up.key)), None, Value::Null);
        }
        last = Some(r.status);
    }
    let code = last.unwrap_or_default();
    (fail(format!("认证失败（HTTP {code}）")), None, Value::Null)
}

/// Anthropic Messages 非流式响应的协议形状；不看 id 前缀、模型名、stop_reason 等只有官方才保证的细节。
fn is_messages_response(v: &Value) -> bool {
    v["type"] == "message" && v["role"] == "assistant" && v["content"].is_array() && v["usage"].is_object()
}

const STREAM_EVENTS: &[&str] = &[
    "message_start",
    "content_block_start",
    "content_block_delta",
    "content_block_stop",
    "message_delta",
    "message_stop",
];

/// SSE 事件名：优先取 `event:` 行；整段都没有 `event:` 行时退回 `data:` JSON 里的 `type`。
fn sse_event_names(text: &str) -> Vec<String> {
    let named: Vec<String> = text
        .lines()
        .filter_map(|l| l.strip_prefix("event:"))
        .map(|n| n.trim().to_string())
        .collect();
    if !named.is_empty() {
        return named;
    }
    text.lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .filter_map(|d| serde_json::from_str::<Value>(d.trim()).ok())
        .filter_map(|v| v["type"].as_str().map(str::to_string))
        .collect()
}

/// 只看 Messages 协议定义的事件，`ping` 等其它事件（中转站常在 message_start 前先发保活）一律忽略：
/// 第一个是 message_start、最后一个是 message_stop 即可。
fn is_messages_stream(events: &[String]) -> bool {
    let known: Vec<&str> = events
        .iter()
        .map(String::as_str)
        .filter(|e| STREAM_EVENTS.contains(e))
        .collect();
    known.first() == Some(&"message_start") && known.last() == Some(&"message_stop")
}

async fn step_format(up: &Upstream<'_>, style: &AuthStyle, model: &str, auth_resp: &Value) -> StepResult {
    if !is_messages_response(auth_resp) {
        return fail(NOT_MESSAGES_FORMAT);
    }
    match up.post("/v1/messages", style, &hello_body(model, true)).await {
        Ok(r) if !r.ok() => fail(http_error(&r, up.key)),
        Ok(r) if is_messages_stream(&sse_event_names(&r.text)) => StepResult::Pass,
        Ok(_) => fail(NOT_MESSAGES_STREAM),
        Err(e) => fail(e),
    }
}

/// 依次执行认证、响应格式两步，每步结果经 `on_step` 回调；第一个 Fail 后停止，两步都 Pass 即通过。
pub async fn verify(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    model: &str,
    on_step: impl Fn(&StepId, &StepResult),
) -> VerifyReport {
    let mut report = VerifyReport {
        steps: Vec::new(),
        auth_style: None,
        ok: false,
        transient: false,
    };
    let record = |report: &mut VerifyReport, id: StepId, result: StepResult| -> bool {
        let result = match result {
            StepResult::Fail { reason } => StepResult::Fail { reason: scrub(&reason, api_key) },
            StepResult::Skip { reason } => StepResult::Skip { reason: scrub(&reason, api_key) },
            StepResult::Pass => StepResult::Pass,
        };
        on_step(&id, &result);
        let passed = !matches!(result, StepResult::Fail { .. });
        report.steps.push((id, result));
        passed
    };

    let checked = super::validate_model(model).and_then(|_| super::normalize_base_url(base_url));
    let base = match checked {
        Ok(b) => b,
        Err(e) => {
            record(&mut report, StepId::Auth, fail(e));
            return report;
        }
    };
    let up = Upstream { client, base, key: api_key, last: AtomicU16::new(200) };
    // 遇到第一个 Fail 即停，此时最后一次请求就是导致失败的那次。
    let done = |mut report: VerifyReport| {
        report.transient = !report.ok && is_transient_status(up.last.load(Ordering::SeqCst));
        report
    };

    let (result, style, auth_resp) = step_auth(&up, model).await;
    if !record(&mut report, StepId::Auth, result) {
        return done(report);
    }
    let style = style.unwrap_or(AuthStyle::XApiKey);
    report.auth_style = Some(style.clone());

    let result = step_format(&up, &style, model, &auth_resp).await;
    report.ok = record(&mut report, StepId::Format, result);
    done(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::{IntoResponse, Response};
    use axum::routing::post;
    use serde_json::{Value, json};
    use std::sync::Arc;

    const KEY: &str = "sk-test-SECRET-key-1234567890";
    const GOOD_SSE: &str = concat!(
        "event: message_start\ndata: {\"type\":\"message_start\"}\n\n",
        "event: content_block_start\ndata: {\"type\":\"content_block_start\"}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\"}\n\n",
        "event: content_block_stop\ndata: {\"type\":\"content_block_stop\"}\n\n",
        "event: message_delta\ndata: {\"type\":\"message_delta\"}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    );

    #[derive(Clone)]
    struct FakeUpstream {
        require_bearer: bool,
        /// 非流式响应体；None 用官方 Claude 形状。
        json_body: Option<Value>,
        /// 流式响应体；None 用 `GOOD_SSE`。
        sse_body: Option<String>,
        /// 流式请求固定返回此 HTTP 状态的错误。
        stream_status: Option<u16>,
        /// 所有 /v1/messages 都返回 400 与此 error.message。
        error_message: Option<String>,
    }

    impl FakeUpstream {
        fn new() -> Self {
            Self { require_bearer: false, json_body: None, sse_body: None, stream_status: None, error_message: None }
        }
        fn with_json(v: Value) -> Self {
            Self { json_body: Some(v), ..Self::new() }
        }
        fn with_sse(s: &str) -> Self {
            Self { sse_body: Some(s.into()), ..Self::new() }
        }
    }

    fn api_error(status: u16, message: &str) -> Response {
        (
            StatusCode::from_u16(status).unwrap(),
            axum::Json(json!({"type":"error","error":{"type":"invalid_request_error","message":message}})),
        )
            .into_response()
    }

    fn authorized(up: &FakeUpstream, h: &HeaderMap) -> bool {
        if up.require_bearer {
            h.get("authorization").and_then(|v| v.to_str().ok()) == Some(&format!("Bearer {KEY}"))
        } else {
            h.get("x-api-key").and_then(|v| v.to_str().ok()) == Some(KEY)
        }
    }

    fn unauthorized() -> Response {
        (
            StatusCode::UNAUTHORIZED,
            axum::Json(json!({"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}})),
        )
            .into_response()
    }

    async fn messages(
        State(up): State<Arc<FakeUpstream>>,
        h: HeaderMap,
        axum::Json(body): axum::Json<Value>,
    ) -> Response {
        if !authorized(&up, &h) {
            return unauthorized();
        }
        if let Some(msg) = &up.error_message {
            return api_error(400, msg);
        }
        if body["stream"] == json!(true) {
            if let Some(status) = up.stream_status {
                return api_error(status, "overloaded");
            }
            let sse = up.sse_body.clone().unwrap_or_else(|| GOOD_SSE.to_string());
            return ([("content-type", "text/event-stream")], sse).into_response();
        }
        let v = up.json_body.clone().unwrap_or_else(|| {
            json!({
                "id": "msg_01ABC",
                "type": "message",
                "role": "assistant",
                "model": body["model"],
                "content": [{"type":"text","text":"OK"}],
                "stop_reason": "end_turn",
                "stop_sequence": null,
                "usage": {"input_tokens": 20, "output_tokens": 3}
            })
        });
        axum::Json(v).into_response()
    }

    async fn serve(up: FakeUpstream) -> String {
        let app = Router::new().route("/v1/messages", post(messages)).with_state(Arc::new(up));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}")
    }

    async fn run(up: FakeUpstream, model: &str) -> VerifyReport {
        let base = serve(up).await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let seen = std::sync::Mutex::new(Vec::new());
        let report = verify(&client, &base, KEY, model, |id, r| {
            seen.lock().unwrap().push((id.clone(), r.clone()));
        })
        .await;
        assert_eq!(*seen.lock().unwrap(), report.steps, "on_step 回调与报告一致");
        let dbg = format!("{report:?}");
        assert!(!dbg.contains(KEY), "{dbg}");
        report
    }

    fn last_fail(r: &VerifyReport) -> (StepId, String) {
        assert!(!r.ok, "{r:?}");
        match r.steps.last().unwrap() {
            (id, StepResult::Fail { reason }) => (id.clone(), reason.clone()),
            other => panic!("最后一步不是 Fail：{other:?}"),
        }
    }

    /// Qwen / DeepSeek 等兼容接口常见的形状：id 不带 msg_、模型名与填写的不同、没有 stop_reason。
    fn compatible_json() -> Value {
        json!({
            "id": "chatcmpl-123",
            "type": "message",
            "role": "assistant",
            "model": "deepseek-chat",
            "content": [{"type":"text","text":"OK"}],
            "usage": {"input_tokens": 9, "output_tokens": 2}
        })
    }

    #[tokio::test]
    async fn claude_shaped_api_passes_two_steps() {
        let r = run(FakeUpstream::new(), "claude-opus-5-5").await;
        assert!(r.ok, "{r:?}");
        assert_eq!(r.steps, [(StepId::Auth, StepResult::Pass), (StepId::Format, StepResult::Pass)]);
        assert_eq!(r.auth_style, Some(AuthStyle::XApiKey));
        assert!(!r.transient);
    }

    #[tokio::test]
    async fn compatible_api_with_any_model_name_passes() {
        for model in ["qwen3-max", "deepseek-chat", "Qwen/Qwen3-Coder", "kimi-k2:latest"] {
            let r = run(FakeUpstream::with_json(compatible_json()), model).await;
            assert!(r.ok, "{model}: {r:?}");
            assert_eq!(r.steps.len(), 2);
        }
    }

    #[tokio::test]
    async fn bearer_fallback_detected() {
        let up = FakeUpstream { require_bearer: true, ..FakeUpstream::new() };
        let r = run(up, "claude-opus-5-5").await;
        assert!(r.ok, "{r:?}");
        assert_eq!(r.auth_style, Some(AuthStyle::Bearer));
    }

    #[tokio::test]
    async fn wrong_key_fails_auth_without_leaking_key() {
        let base = serve(FakeUpstream::new()).await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let r = verify(&client, &base, "sk-wrong-key-abcdefghijk", "claude-opus-5-5", |_, _| {}).await;
        assert!(!r.ok);
        assert_eq!(r.auth_style, None);
        assert_eq!(r.steps, [(StepId::Auth, StepResult::Fail { reason: "认证失败（HTTP 401）".into() })]);
    }

    #[tokio::test]
    async fn non_messages_json_fails_format() {
        let openai = json!({
            "id": "chatcmpl-1",
            "object": "chat.completion",
            "choices": [{"message": {"role": "assistant", "content": "OK"}}],
            "usage": {"prompt_tokens": 9, "completion_tokens": 2}
        });
        let mut no_usage = compatible_json();
        no_usage.as_object_mut().unwrap().remove("usage");
        let mut text_content = compatible_json();
        text_content["content"] = json!("OK");
        let mut user_role = compatible_json();
        user_role["role"] = json!("user");
        for body in [openai, no_usage, text_content, user_role, json!("not an object")] {
            let r = run(FakeUpstream::with_json(body.clone()), "qwen3-max").await;
            assert_eq!(last_fail(&r), (StepId::Format, NOT_MESSAGES_FORMAT.into()), "{body}");
            assert_eq!(r.steps.len(), 2);
            assert!(!r.transient);
        }
    }

    #[tokio::test]
    async fn wrong_sse_sequence_fails_format() {
        let missing_stop = GOOD_SSE.replace("event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n", "");
        let openai_sse = "data: {\"choices\":[{\"delta\":{\"content\":\"OK\"}}]}\n\ndata: [DONE]\n\n";
        for sse in [missing_stop.as_str(), openai_sse, ""] {
            let r = run(FakeUpstream::with_sse(sse), "claude-opus-5-5").await;
            assert_eq!(last_fail(&r), (StepId::Format, NOT_MESSAGES_STREAM.into()), "{sse:?}");
        }
    }

    #[tokio::test]
    async fn sse_without_event_lines_uses_data_type() {
        let data_only: String = GOOD_SSE.lines().filter(|l| !l.starts_with("event:")).map(|l| format!("{l}\n")).collect();
        assert!(run(FakeUpstream::with_sse(&data_only), "deepseek-chat").await.ok);
    }

    #[test]
    fn stream_check_ignores_unknown_events() {
        let ev = |names: &[&str]| names.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // 中转站常在 message_start 前先发 ping 保活（UCloud modelverse 实测，2026-11-18）。
        assert!(is_messages_stream(&ev(&[
            "ping", "message_start", "content_block_start", "ping", "message_delta", "message_stop", "ping"
        ])));
        assert!(is_messages_stream(&ev(&["message_start", "vendor_usage", "message_stop"])));
        assert!(!is_messages_stream(&ev(&["ping", "content_block_start", "message_delta", "message_stop"])));
        assert!(!is_messages_stream(&ev(&["message_start", "content_block_start", "message_delta", "ping"])));
        assert!(!is_messages_stream(&ev(&[])));
    }

    #[tokio::test]
    async fn key_echoed_across_truncation_point_is_scrubbed() {
        let up = FakeUpstream { error_message: Some(format!("{}{KEY} rest", "x".repeat(195))), ..FakeUpstream::new() };
        let r = run(up, "claude-opus-5-5").await;
        let (id, reason) = last_fail(&r);
        assert_eq!(id, StepId::Auth);
        assert!(reason.starts_with("接口返回 HTTP 400："), "{reason}");
        assert!(!reason.contains(&KEY[..10]), "{reason}");
    }

    #[tokio::test]
    async fn invalid_input_fails_auth_without_request() {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        // 不起假上游：一旦发请求就会连接失败，文案会不同。
        let r = verify(&client, "http://127.0.0.1:9", KEY, "bad model", |_, _| {}).await;
        let (id, reason) = last_fail(&r);
        assert_eq!(id, StepId::Auth);
        assert_eq!(reason, crate::proxy::custom_api::validate_model("bad model").unwrap_err());
        assert!(!r.transient);
        let r = verify(&client, "api.x.com", KEY, "claude-opus-5-5", |_, _| {}).await;
        assert_eq!(last_fail(&r), (StepId::Auth, "Base URL 必须以 http:// 或 https:// 开头".into()));
        assert!(!r.transient);
    }

    #[test]
    fn transient_status_classification() {
        for s in [0, 429, 500, 502, 503, 529] {
            assert!(is_transient_status(s), "{s}");
        }
        for s in [200, 400, 401, 403, 404, 422] {
            assert!(!is_transient_status(s), "{s}");
        }
    }

    #[tokio::test]
    async fn report_marks_network_failures_transient() {
        // 上游 5xx / 429：网络类
        for status in [500u16, 503, 429] {
            let up = FakeUpstream { stream_status: Some(status), ..FakeUpstream::new() };
            let r = run(up, "claude-opus-5-5").await;
            assert!(!r.ok && r.transient, "{status}: {r:?}");
            assert_eq!(last_fail(&r).1, format!("接口返回 HTTP {status}：overloaded"));
        }
        // 连接失败：网络类
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let r = verify(&client, "http://127.0.0.1:9", KEY, "claude-opus-5-5", |_, _| {}).await;
        assert!(!r.ok && r.transient, "{r:?}");
        // 判定类：格式不对、流式 400、认证 401
        let bad_stream_400 = FakeUpstream { stream_status: Some(400), ..FakeUpstream::new() };
        for up in [FakeUpstream::with_json(json!({"object": "chat.completion"})), bad_stream_400] {
            let r = run(up, "claude-opus-5-5").await;
            assert!(!r.ok && !r.transient, "{r:?}");
        }
        let base = serve(FakeUpstream::new()).await;
        let r = verify(&client, &base, "sk-wrong-key-abcdefghijk", "claude-opus-5-5", |_, _| {}).await;
        assert!(!r.ok && !r.transient, "{r:?}");
    }

    #[test]
    fn scrub_masks_key_in_reason() {
        let s = scrub(&format!("bad key {KEY}"), KEY);
        assert!(!s.contains(KEY), "{s}");
    }

    #[test]
    fn step_result_serializes_with_kind_tag() {
        assert_eq!(serde_json::to_value(StepResult::Pass).unwrap(), json!({"kind":"pass"}));
        assert_eq!(
            serde_json::to_value(StepResult::Fail { reason: "x".into() }).unwrap(),
            json!({"kind":"fail","reason":"x"})
        );
        assert_eq!(serde_json::to_value(StepId::Auth).unwrap(), json!("auth"));
        assert_eq!(serde_json::to_value(StepId::Format).unwrap(), json!("format"));
    }
}
