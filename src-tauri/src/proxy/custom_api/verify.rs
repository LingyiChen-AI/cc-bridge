//! 真 Claude 校验：认证 → 格式 → 分词指纹 → 签名篡改 → 模型名，遇到第一个 Fail 即停。
//! 宁可错拒不可错放：最强的一步是篡改 thinking 块签名后回放，只有真 Anthropic 会校验签名并返回 400。
//! key 只出现在出站请求头里；所有返回的文案都经 `scrub` 把 key 换成掩码。

use reqwest::header::HeaderValue;
use serde::Serialize;
use serde_json::{Value, json};
use std::time::Duration;

use super::fingerprint::{FIXED_TEXT, baseline};
use super::{AuthStyle, mask_key};

const TIMEOUT: Duration = Duration::from_secs(30);
const QUESTION: &str =
    "A train leaves at 9:47 and arrives at 13:12. How many minutes is the trip? Reason carefully.";
const FALLBACK_QUESTION: &str = "Is 391 prime? Check divisors step by step.";

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum StepId {
    Auth,
    Format,
    Tokenizer,
    Signature,
    ModelName,
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

/// 倒数第 8 个字符：是 `A` 换 `B`，否则换 `A`（不足 8 个字符时改第一个）。
pub(crate) fn tamper_signature(sig: &str) -> String {
    let mut chars: Vec<char> = sig.chars().collect();
    if chars.is_empty() {
        return "A".into();
    }
    let i = chars.len().saturating_sub(8);
    chars[i] = if chars[i] == 'A' { 'B' } else { 'A' };
    chars.into_iter().collect()
}

/// reqwest 错误不带 URL、不用 Debug，只拼 Display 与 source 链。
fn describe_error(err: reqwest::Error) -> String {
    let err = err.without_url();
    let mut out = err.to_string();
    let mut src = std::error::Error::source(&err);
    while let Some(s) = src {
        out.push_str("：");
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
    /// `error.message`，没有就用原文；取前 200 字。
    fn error_message(&self) -> String {
        let v = self.json();
        let msg = v["error"]["message"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| self.text.clone());
        msg.chars().take(200).collect()
    }
}

struct Upstream<'a> {
    client: &'a reqwest::Client,
    base: String,
    key: &'a str,
}

impl Upstream<'_> {
    async fn post(&self, path: &str, style: &AuthStyle, body: &Value) -> Result<Resp, String> {
        let (name, raw) = match style {
            AuthStyle::XApiKey => ("x-api-key", self.key.to_string()),
            AuthStyle::Bearer => ("authorization", format!("Bearer {}", self.key)),
        };
        let mut auth = HeaderValue::from_str(&raw).map_err(|_| "API Key 含有非法字符".to_string())?;
        auth.set_sensitive(true);
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
        Ok(Resp { status, text })
    }
}

fn http_error(r: &Resp) -> String {
    format!("接口返回 HTTP {}：{}", r.status, r.error_message())
}

async fn step_auth(up: &Upstream<'_>, model: &str) -> (StepResult, Option<AuthStyle>, Value) {
    let body = json!({
        "model": model,
        "max_tokens": 16,
        "messages": [{"role": "user", "content": "Reply with OK."}],
    });
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
            return (fail(http_error(&r)), None, Value::Null);
        }
        last = Some(r.status);
    }
    let code = last.unwrap_or_default();
    (fail(format!("认证失败（HTTP {code}）")), None, Value::Null)
}

fn is_claude_message(v: &Value) -> bool {
    v["type"] == "message"
        && v["id"].as_str().is_some_and(|s| s.starts_with("msg_"))
        && v["role"] == "assistant"
        && v["content"].is_array()
        && v["usage"]["input_tokens"].as_u64().is_some()
        && v["usage"]["output_tokens"].as_u64().is_some()
        && v["stop_reason"].as_str().is_some_and(|s| !s.is_empty())
}

fn sse_event_names(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.strip_prefix("event:"))
        .map(|n| n.trim().to_string())
        .collect()
}

fn is_claude_stream(events: &[String]) -> bool {
    events.first().is_some_and(|e| e == "message_start")
        && events.iter().any(|e| e == "content_block_start")
        && events.iter().any(|e| e == "message_delta")
        && events.last().is_some_and(|e| e == "message_stop")
}

async fn step_format(up: &Upstream<'_>, style: &AuthStyle, model: &str, auth_resp: &Value) -> StepResult {
    const NOT_CLAUDE: &str = "不是 Claude 格式的接口";
    if !is_claude_message(auth_resp) {
        return fail(NOT_CLAUDE);
    }
    let body = json!({
        "model": model,
        "max_tokens": 16,
        "stream": true,
        "messages": [{"role": "user", "content": "Reply with OK."}],
    });
    match up.post("/v1/messages", style, &body).await {
        Ok(r) if r.ok() && is_claude_stream(&sse_event_names(&r.text)) => StepResult::Pass,
        Ok(_) => fail(NOT_CLAUDE),
        Err(e) => fail(e),
    }
}

async fn step_tokenizer(up: &Upstream<'_>, style: &AuthStyle, model: &str) -> StepResult {
    let Some(base) = baseline(model) else {
        return StepResult::Skip {
            reason: "该型号没有分词基准，仅凭签名判断".into(),
        };
    };
    let messages = json!([{"role": "user", "content": FIXED_TEXT}]);
    let counted = match up
        .post(
            "/v1/messages/count_tokens",
            style,
            &json!({"model": model, "messages": messages}),
        )
        .await
    {
        Ok(r) if r.status == 200 => r.json()["input_tokens"].as_u64(),
        Ok(_) => None,
        Err(e) => return fail(e),
    };
    if counted == Some(u64::from(base.count_tokens)) {
        return StepResult::Pass;
    }
    let (got, want) = match base.usage_input {
        Some(usage_want) => {
            let used = match up
                .post(
                    "/v1/messages",
                    style,
                    &json!({"model": model, "max_tokens": 16, "messages": messages}),
                )
                .await
            {
                Ok(r) if r.ok() => r.json()["usage"]["input_tokens"].as_u64(),
                Ok(_) => None,
                Err(e) => return fail(e),
            };
            if used == Some(u64::from(usage_want)) {
                return StepResult::Pass;
            }
            (used, usage_want)
        }
        None => (counted, base.count_tokens),
    };
    let got = got.map_or_else(|| "无法获取".to_string(), |n| n.to_string());
    fail(format!("分词结果和 Claude 不一致（得到 {got}，应为 {want}），可能不是 Claude 模型"))
}

/// 老一代（4.x）用 budget_tokens，其余用 adaptive。返回 (max_tokens, 额外字段)。
fn thinking_params(model: &str) -> (u32, Vec<(&'static str, Value)>) {
    let legacy = ["claude-haiku-4-5", "claude-sonnet-4-", "claude-opus-4-"]
        .iter()
        .any(|p| model.starts_with(p));
    if legacy {
        (2048, vec![("thinking", json!({"type": "enabled", "budget_tokens": 1024}))])
    } else {
        (
            4096,
            vec![
                ("thinking", json!({"type": "adaptive", "display": "omitted"})),
                ("output_config", json!({"effort": "high"})),
            ],
        )
    }
}

fn thinking_body(model: &str, messages: Value) -> Value {
    let (max_tokens, extra) = thinking_params(model);
    let mut body = json!({"model": model, "max_tokens": max_tokens, "messages": messages});
    for (k, v) in extra {
        body[k] = v;
    }
    body
}

/// 找出带非空签名的 thinking / redacted_thinking 块及全部 text 块。
fn signed_block(resp: &Value) -> Option<(Value, Vec<Value>)> {
    let content = resp["content"].as_array()?;
    let block = content.iter().find(|b| {
        (b["type"] == "thinking" || b["type"] == "redacted_thinking")
            && b["signature"].as_str().is_some_and(|s| !s.is_empty())
    })?;
    let texts = content.iter().filter(|b| b["type"] == "text").cloned().collect();
    Some((block.clone(), texts))
}

async fn step_signature(up: &Upstream<'_>, style: &AuthStyle, model: &str) -> StepResult {
    let mut found = None;
    for q in [QUESTION, FALLBACK_QUESTION] {
        let body = thinking_body(model, json!([{"role": "user", "content": q}]));
        match up.post("/v1/messages", style, &body).await {
            Ok(r) if r.ok() => {
                if let Some(hit) = signed_block(&r.json()) {
                    found = Some((q, hit));
                    break;
                }
            }
            Ok(_) => {}
            Err(e) => return fail(e),
        }
    }
    let Some((question, (mut block, texts))) = found else {
        return fail("接口不支持 Claude 的思考签名");
    };
    let tampered = tamper_signature(block["signature"].as_str().unwrap_or_default());
    block["signature"] = Value::String(tampered);
    let mut assistant = vec![block];
    assistant.extend(texts);
    let body = thinking_body(
        model,
        json!([
            {"role": "user", "content": question},
            {"role": "assistant", "content": assistant},
            {"role": "user", "content": "Now add 1."},
        ]),
    );
    match up.post("/v1/messages", style, &body).await {
        Ok(r) if r.status == 400
            && r.json()["error"]["message"]
                .as_str()
                .is_some_and(|m| m.to_ascii_lowercase().contains("signature")) =>
        {
            StepResult::Pass
        }
        Ok(r) if r.ok() => fail("签名没有被校验，不是真正的 Claude"),
        Ok(r) => fail(format!("签名测试异常：HTTP {}", r.status)),
        Err(e) => fail(format!("签名测试异常：{e}")),
    }
}

fn step_model_name(model: &str, auth_resp: &Value) -> StepResult {
    let echoed = auth_resp["model"].as_str().unwrap_or_default();
    if echoed == model || echoed.starts_with(&format!("{model}-")) {
        StepResult::Pass
    } else {
        fail(format!("接口返回的模型是 {echoed}，和填写的 {model} 不一致"))
    }
}

/// 依次执行五步，每步结果经 `on_step` 回调；第一个 Fail 后停止。
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

    let base = match super::normalize_base_url(base_url) {
        Ok(b) => b,
        Err(e) => {
            record(&mut report, StepId::Auth, fail(e));
            return report;
        }
    };
    let up = Upstream { client, base, key: api_key };

    let (result, style, auth_resp) = step_auth(&up, model).await;
    if !record(&mut report, StepId::Auth, result) {
        return report;
    }
    let style = style.unwrap_or(AuthStyle::XApiKey);
    report.auth_style = Some(style.clone());

    let result = step_format(&up, &style, model, &auth_resp).await;
    if !record(&mut report, StepId::Format, result) {
        return report;
    }
    let result = step_tokenizer(&up, &style, model).await;
    if !record(&mut report, StepId::Tokenizer, result) {
        return report;
    }
    let result = step_signature(&up, &style, model).await;
    if !record(&mut report, StepId::Signature, result) {
        return report;
    }
    let result = step_model_name(model, &auth_resp);
    report.ok = record(&mut report, StepId::ModelName, result);
    report
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
    const SIG: &str = "EqQBCkYIBhABGAIiQGenuineSignatureForTestingOnly1234567890abcdefgh";

    #[derive(Clone)]
    struct FakeUpstream {
        genuine: bool,
        count_tokens: u32,
        model_echo: String,
        require_bearer: bool,
    }

    impl FakeUpstream {
        fn new(model_echo: &str) -> Self {
            Self {
                genuine: true,
                count_tokens: 68,
                model_echo: model_echo.into(),
                require_bearer: false,
            }
        }
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
        if body["stream"] == json!(true) {
            let sse = concat!(
                "event: message_start\ndata: {\"type\":\"message_start\"}\n\n",
                "event: content_block_start\ndata: {\"type\":\"content_block_start\"}\n\n",
                "event: content_block_delta\ndata: {\"type\":\"content_block_delta\"}\n\n",
                "event: content_block_stop\ndata: {\"type\":\"content_block_stop\"}\n\n",
                "event: message_delta\ndata: {\"type\":\"message_delta\"}\n\n",
                "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
            );
            return ([("content-type", "text/event-stream")], sse).into_response();
        }
        let replayed = body["messages"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|m| m["role"] == "assistant")
            .filter_map(|m| m["content"].as_array())
            .flatten()
            .find(|b| b["type"] == "thinking")
            .and_then(|b| b["signature"].as_str().map(str::to_string));
        if up.genuine && replayed.as_deref().is_some_and(|s| s != SIG) {
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(json!({"type":"error","error":{"type":"invalid_request_error","message":"messages.1.content.0: Invalid `signature` in `thinking` block"}})),
            )
                .into_response();
        }
        let mut content = Vec::new();
        if body.get("thinking").is_some() {
            content.push(json!({"type":"thinking","thinking":"","signature":SIG}));
        }
        content.push(json!({"type":"text","text":"OK"}));
        axum::Json(json!({
            "id": "msg_01ABC",
            "type": "message",
            "role": "assistant",
            "model": up.model_echo,
            "content": content,
            "stop_reason": "end_turn",
            "stop_sequence": null,
            "usage": {"input_tokens": 20, "output_tokens": 3}
        }))
        .into_response()
    }

    async fn count_tokens(State(up): State<Arc<FakeUpstream>>, h: HeaderMap) -> Response {
        if !authorized(&up, &h) {
            return unauthorized();
        }
        axum::Json(json!({"input_tokens": up.count_tokens})).into_response()
    }

    async fn serve(up: FakeUpstream) -> String {
        let app = Router::new()
            .route("/v1/messages", post(messages))
            .route("/v1/messages/count_tokens", post(count_tokens))
            .with_state(Arc::new(up));
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

    fn ids(r: &VerifyReport) -> Vec<StepId> {
        r.steps.iter().map(|(id, _)| id.clone()).collect()
    }

    #[tokio::test]
    async fn genuine_claude_passes_all_steps() {
        let r = run(FakeUpstream::new("claude-opus-5-5"), "claude-opus-5-5").await;
        assert!(r.ok, "{r:?}");
        assert_eq!(
            ids(&r),
            [StepId::Auth, StepId::Format, StepId::Tokenizer, StepId::Signature, StepId::ModelName]
        );
        assert!(r.steps.iter().all(|(_, s)| *s == StepResult::Pass), "{r:?}");
        assert_eq!(r.auth_style, Some(AuthStyle::XApiKey));
    }

    #[tokio::test]
    async fn compatible_api_fails_on_signature() {
        let mut up = FakeUpstream::new("claude-opus-5-5");
        up.genuine = false;
        let r = run(up, "claude-opus-5-5").await;
        assert!(!r.ok);
        assert_eq!(r.steps.len(), 4);
        assert_eq!(r.steps[3].0, StepId::Signature);
        assert_eq!(
            r.steps[3].1,
            StepResult::Fail { reason: "签名没有被校验，不是真正的 Claude".into() }
        );
    }

    #[tokio::test]
    async fn wrong_tokenizer_fails_before_signature() {
        let mut up = FakeUpstream::new("claude-opus-5-5");
        up.count_tokens = 55;
        let r = run(up, "claude-opus-5-5").await;
        assert!(!r.ok);
        assert_eq!(r.steps.len(), 3);
        assert_eq!(r.steps[2].0, StepId::Tokenizer);
        assert_eq!(
            r.steps[2].1,
            StepResult::Fail {
                reason: "分词结果和 Claude 不一致（得到 55，应为 68），可能不是 Claude 模型".into()
            }
        );
    }

    #[tokio::test]
    async fn unknown_model_skips_tokenizer() {
        let r = run(FakeUpstream::new("claude-opus-9"), "claude-opus-9").await;
        assert!(r.ok, "{r:?}");
        assert_eq!(r.steps.len(), 5);
        for (id, s) in &r.steps {
            if *id == StepId::Tokenizer {
                assert_eq!(*s, StepResult::Skip { reason: "该型号没有分词基准，仅凭签名判断".into() });
            } else {
                assert_eq!(*s, StepResult::Pass, "{id:?}");
            }
        }
    }

    #[tokio::test]
    async fn bearer_fallback_detected() {
        let mut up = FakeUpstream::new("claude-opus-5-5");
        up.require_bearer = true;
        let r = run(up, "claude-opus-5-5").await;
        assert!(r.ok, "{r:?}");
        assert_eq!(r.auth_style, Some(AuthStyle::Bearer));
    }

    #[tokio::test]
    async fn model_echo_mismatch_fails() {
        let r = run(FakeUpstream::new("claude-sonnet-5-5"), "claude-opus-5-5").await;
        assert!(!r.ok);
        assert_eq!(r.steps.len(), 5);
        assert_eq!(
            r.steps[4],
            (
                StepId::ModelName,
                StepResult::Fail {
                    reason: "接口返回的模型是 claude-sonnet-5-5，和填写的 claude-opus-5-5 不一致".into()
                }
            )
        );
    }

    #[tokio::test]
    async fn dated_echo_is_accepted() {
        let mut up = FakeUpstream::new("claude-haiku-4-5-20251001");
        up.count_tokens = 60;
        let r = run(up, "claude-haiku-4-5").await;
        assert!(r.ok, "{r:?}");
        assert_eq!(r.steps[4], (StepId::ModelName, StepResult::Pass));
    }

    #[tokio::test]
    async fn wrong_key_fails_auth_without_leaking_key() {
        let base = serve(FakeUpstream::new("claude-opus-5-5")).await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let r = verify(&client, &base, "sk-wrong-key-abcdefghijk", "claude-opus-5-5", |_, _| {}).await;
        assert!(!r.ok);
        assert_eq!(r.auth_style, None);
        assert_eq!(
            r.steps,
            [(StepId::Auth, StepResult::Fail { reason: "认证失败（HTTP 401）".into() })]
        );
    }

    #[test]
    fn tamper_changes_eighth_from_last_char() {
        assert_eq!(tamper_signature("abcdefghijkl"), "abcdAfghijkl");
        assert_eq!(tamper_signature("abcdAfghijkl"), "abcdBfghijkl");
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
        assert_eq!(serde_json::to_value(StepId::ModelName).unwrap(), json!("modelName"));
    }
}
