//! 真 Claude 校验：认证 → 格式 → 分词指纹 → 签名篡改 → 模型名，遇到第一个 Fail 即停。
//! 宁可错拒不可错放：最强的一步是篡改 thinking 块签名后回放，只有真 Anthropic 会校验签名并返回 400；
//! 中转站吞掉 400、剥掉 thinking 重试时，靠回放前后总输入（含缓存字段）明显下降判定（见 `stripped_by_relay`）；
//! 该判据只在分词步骤 Pass 时启用，且篡改回放先于原签名回放发出（前缀缓存只会让判据偏向拒绝）。
//! key 只出现在出站请求头里；所有返回的文案都经 `scrub` 把 key 换成掩码。

use reqwest::header::HeaderValue;
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;

use super::fingerprint::{Baseline, FIXED_TEXT, baseline};
use super::{AuthStyle, mask_key};

const TIMEOUT: Duration = Duration::from_secs(30);
const QUESTION: &str =
    "A train leaves at 9:47 and arrives at 13:12. How many minutes is the trip? Reason carefully.";
const FALLBACK_QUESTION: &str = "Is 391 prime? Check divisors step by step.";
/// 篡改回放 2xx 且分词步骤被跳过时的文案：没有分词基准就不信任用量差分。
pub(crate) const SKIP_TOKENIZER_DIFF_FAIL: &str =
    "签名没有被校验，不是真正的 Claude（该型号无分词基准时只接受签名报错判据）";

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
    /// 失败是网络类（请求失败 / 超时、HTTP 5xx、429），不是对接口真伪的判定；重新校验时据此保留已验证状态。
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
            return (fail(http_error(&r, up.key)), None, Value::Null);
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

/// `ping` 是 Anthropic SSE 的保活事件，可出现在任意位置（中转站常在 message_start 前先发），判断顺序时忽略。
fn is_claude_stream(events: &[String]) -> bool {
    let events: Vec<&String> = events.iter().filter(|e| *e != "ping").collect();
    events.first().is_some_and(|e| *e == "message_start")
        && events.iter().any(|e| *e == "content_block_start")
        && events.iter().any(|e| *e == "message_delta")
        && events.last().is_some_and(|e| *e == "message_stop")
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

async fn step_tokenizer(
    up: &Upstream<'_>,
    style: &AuthStyle,
    model: &str,
    base: Option<Baseline>,
) -> StepResult {
    let Some(base) = base else {
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

/// `tokenizer_passed`：分词步骤为 Pass（不是 Skip）时才允许走差分判据。
async fn step_signature(up: &Upstream<'_>, style: &AuthStyle, model: &str, tokenizer_passed: bool) -> StepResult {
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
    let Some((question, (block, texts))) = found else {
        return fail("接口不支持 Claude 的思考签名");
    };
    let replay = |block: Value| {
        let mut assistant = vec![block];
        assistant.extend(texts.iter().cloned());
        thinking_body(
            model,
            json!([
                {"role": "user", "content": question},
                {"role": "assistant", "content": assistant},
                {"role": "user", "content": "Now add 1."},
            ]),
        )
    };

    // 先发篡改回放、后发原签名回放：两次前缀相同，前缀缓存（哪怕用非标准字段上报）只会让
    // 后发的原签名回放（V）变小，差分判据只会偏向拒绝。
    let mut tampered = block.clone();
    let sig = tamper_signature(tampered["signature"].as_str().unwrap_or_default());
    tampered["signature"] = Value::String(sig);
    let tampered_resp = match up.post("/v1/messages", style, &replay(tampered)).await {
        Ok(r) => r,
        Err(e) => return fail(format!("签名测试异常：{e}")),
    };
    let signature_error = tampered_resp.status == 400
        && tampered_resp.json()["error"]["message"]
            .as_str()
            .is_some_and(|m| m.to_ascii_lowercase().contains("signature"));
    if !signature_error && !tampered_resp.ok() {
        return fail(format!("签名测试异常：HTTP {}", tampered_resp.status));
    }

    // 对照：原签名回放必须成功，排除「任何回放都以含 signature 的 400 拒绝」的接口；
    // 其总输入作为差分判据的基准。签名报错判据也要在对照成功后才算数。
    let original_tokens = match up.post("/v1/messages", style, &replay(block)).await {
        Ok(r) if r.ok() => match total_input(&r.json()) {
            Some(n) => n,
            None => return fail("签名测试异常：原签名回放缺少用量"),
        },
        Ok(r) => return fail(format!("签名测试异常：原签名回放返回 HTTP {}", r.status)),
        Err(e) => return fail(format!("签名测试异常：{e}")),
    };

    if signature_error {
        return StepResult::Pass;
    }
    if !tokenizer_passed {
        return fail(SKIP_TOKENIZER_DIFF_FAIL);
    }
    if stripped_by_relay(original_tokens, total_input(&tampered_resp.json())) {
        StepResult::Pass
    } else {
        fail("签名没有被校验，不是真正的 Claude")
    }
}

/// 总输入 = input_tokens + cache_read_input_tokens + cache_creation_input_tokens（缓存字段缺省或 null 为 0）。
/// Anthropic 协议里 input_tokens 不含缓存部分；只比 input_tokens 时，带前缀缓存的非 Claude 接口
/// 会因篡改回放命中缓存而骤降，被误判为「剥掉了 thinking」。input_tokens 本身必须存在；
/// 缓存字段存在却不是非负整数、或相加溢出，都返回 None（宁可判不了也不按 0 算）。
fn total_input(resp: &Value) -> Option<u64> {
    let usage = &resp["usage"];
    let mut total = usage["input_tokens"].as_u64()?;
    for k in ["cache_read_input_tokens", "cache_creation_input_tokens"] {
        let n = match &usage[k] {
            Value::Null => 0,
            v => v.as_u64()?,
        };
        total = total.checked_add(n)?;
    }
    Some(total)
}

/// 差分判据：中转把篡改签名的请求转给 Anthropic 被拒后，常剥掉 thinking 块重试并返回 200。
/// 此时篡改回放的总输入（T，含缓存字段）明显少于原签名回放（V）：T <= V - max(16, V/10) 视为签名被校验过。
/// 不校验签名的接口两次计数相同（或接近）。依赖中转如实上报 usage。
fn stripped_by_relay(original: u64, tampered: Option<u64>) -> bool {
    let margin = (original / 10).max(16);
    match (original.checked_sub(margin), tampered) {
        (Some(limit), Some(t)) => t <= limit,
        _ => false,
    }
}

fn step_model_name(model: &str, auth_resp: &Value) -> StepResult {
    let echoed = auth_resp["model"].as_str().unwrap_or_default();
    let dated = echoed
        .strip_prefix(model)
        .and_then(|rest| rest.strip_prefix('-'))
        .is_some_and(|d| d.len() == 8 && d.bytes().all(|b| b.is_ascii_digit()));
    if echoed == model || dated {
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
    if !record(&mut report, StepId::Format, result) {
        return done(report);
    }
    let result = step_tokenizer(&up, &style, model, baseline(model)).await;
    let tokenizer_passed = result == StepResult::Pass;
    if !record(&mut report, StepId::Tokenizer, result) {
        return done(report);
    }
    let result = step_signature(&up, &style, model, tokenizer_passed).await;
    if !record(&mut report, StepId::Signature, result) {
        return done(report);
    }
    let result = step_model_name(model, &auth_resp);
    report.ok = record(&mut report, StepId::ModelName, result);
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
    const SIG: &str = "EqQBCkYIBhABGAIiQGenuineSignatureForTestingOnly1234567890abcdefgh";

    #[derive(Clone)]
    struct FakeUpstream {
        genuine: bool,
        count_tokens: u32,
        model_echo: String,
        require_bearer: bool,
        /// 非流式响应里的 `usage.input_tokens`。
        usage_input: u64,
        /// 非流式响应不是 Claude 形状（id 不以 msg_ 开头）。
        bad_json: bool,
        /// SSE 缺 `message_stop`。
        bad_sse: bool,
        /// thinking 块不带签名。
        omit_signature: bool,
        /// 篡改签名的回放固定返回此 (状态码, error.message)，优先于 genuine。
        tampered_reply: Option<(u16, &'static str)>,
        /// 任何带 thinking 历史的回放都以含 signature 的 400 拒绝（含原签名）。
        reject_all_replays: bool,
        /// 所有 /v1/messages 都返回 400 与此 error.message。
        error_message: Option<String>,
        /// 中转模式：(原签名回放的 input_tokens, 篡改回放的 input_tokens)，None 表示响应里不带 usage；
        /// 篡改回放返回 200（模拟中转被拒后剥掉 thinking 重试），优先于 genuine。
        replay_tokens: Option<(Option<u64>, Option<u64>)>,
        /// 篡改回放额外报的 `cache_read_input_tokens`（模拟有前缀缓存的非 Claude 接口）。
        tampered_cache_read: Option<u64>,
        /// 合并进篡改回放 usage 的额外字段（模拟格式异常的缓存字段）。
        tampered_usage_extra: Option<Value>,
        /// 不校验签名、带前缀缓存且用非标准字段报缓存的接口：(总输入, 缓存命中数)。
        /// 第一次回放报完整 input_tokens，之后的回放 input_tokens 扣掉缓存命中、命中数放在
        /// OpenAI 风格的 `prompt_tokens_details.cached_tokens`（total_input 不认）。
        prefix_cache: Option<(u64, u64)>,
        /// 已收到的回放请求数（prefix_cache 用）。
        replays_seen: Arc<std::sync::atomic::AtomicUsize>,
        /// 按到达顺序记录回放：true 为篡改签名，false 为原签名。
        replay_order: Arc<std::sync::Mutex<Vec<bool>>>,
    }

    impl FakeUpstream {
        fn new(model_echo: &str) -> Self {
            Self {
                genuine: true,
                count_tokens: 68,
                model_echo: model_echo.into(),
                require_bearer: false,
                usage_input: 20,
                bad_json: false,
                bad_sse: false,
                omit_signature: false,
                tampered_reply: None,
                reject_all_replays: false,
                error_message: None,
                replay_tokens: None,
                tampered_cache_read: None,
                tampered_usage_extra: None,
                prefix_cache: None,
                replays_seen: Arc::default(),
                replay_order: Arc::default(),
            }
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
            let mut sse = concat!(
                "event: message_start\ndata: {\"type\":\"message_start\"}\n\n",
                "event: content_block_start\ndata: {\"type\":\"content_block_start\"}\n\n",
                "event: content_block_delta\ndata: {\"type\":\"content_block_delta\"}\n\n",
                "event: content_block_stop\ndata: {\"type\":\"content_block_stop\"}\n\n",
                "event: message_delta\ndata: {\"type\":\"message_delta\"}\n\n",
            )
            .to_string();
            if !up.bad_sse {
                sse.push_str("event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n");
            }
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
        if up.reject_all_replays && replayed.is_some() {
            return api_error(400, "thinking signature field is not supported");
        }
        let tampered = replayed.as_deref().is_some_and(|s| s != SIG);
        if replayed.is_some() {
            up.replay_order.lock().unwrap().push(tampered);
        }
        if let (Some((full, cached)), Some(_)) = (up.prefix_cache, &replayed) {
            let first = up.replays_seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0;
            let mut v = json!({
                "id": "msg_01CACHE",
                "type": "message",
                "role": "assistant",
                "model": "claude-opus-5-5",
                "content": [{"type":"text","text":"OK"}],
                "stop_reason": "end_turn",
                "usage": {"input_tokens": full, "output_tokens": 3},
            });
            if !first {
                v["usage"]["input_tokens"] = json!(full - cached);
                v["usage"]["prompt_tokens_details"] = json!({"cached_tokens": cached});
            }
            return axum::Json(v).into_response();
        }
        if tampered {
            if let Some((status, msg)) = up.tampered_reply {
                return api_error(status, msg);
            }
            if let Some((_, t)) = up.replay_tokens {
                let mut resp = replay_json(t, up.tampered_cache_read);
                if let Some(Value::Object(extra)) = &up.tampered_usage_extra {
                    for (k, v) in extra {
                        resp["usage"][k] = v.clone();
                    }
                }
                return axum::Json(resp).into_response();
            }
            if up.genuine {
                return api_error(400, "messages.1.content.0: Invalid `signature` in `thinking` block");
            }
        }
        if let (Some((orig, _)), Some(_)) = (up.replay_tokens, &replayed) {
            return replay_ok(orig, None);
        }
        let mut content = Vec::new();
        if body.get("thinking").is_some() {
            if up.omit_signature {
                content.push(json!({"type":"thinking","thinking":"hmm"}));
            } else {
                content.push(json!({"type":"thinking","thinking":"","signature":SIG}));
            }
        }
        content.push(json!({"type":"text","text":"OK"}));
        axum::Json(json!({
            "id": if up.bad_json { "chatcmpl-1" } else { "msg_01ABC" },
            "type": "message",
            "role": "assistant",
            "model": up.model_echo,
            "content": content,
            "stop_reason": "end_turn",
            "stop_sequence": null,
            "usage": {"input_tokens": up.usage_input, "output_tokens": 3}
        }))
        .into_response()
    }

    fn replay_ok(input_tokens: Option<u64>, cache_read: Option<u64>) -> Response {
        axum::Json(replay_json(input_tokens, cache_read)).into_response()
    }

    fn replay_json(input_tokens: Option<u64>, cache_read: Option<u64>) -> Value {
        let mut v = json!({
            "id": "msg_01REPLAY",
            "type": "message",
            "role": "assistant",
            "model": "claude-opus-5-5",
            "content": [{"type":"text","text":"OK"}],
            "stop_reason": "end_turn",
        });
        if let Some(n) = input_tokens {
            v["usage"] = json!({"input_tokens": n, "output_tokens": 3});
            if let Some(c) = cache_read {
                v["usage"]["cache_read_input_tokens"] = json!(c);
                v["usage"]["cache_creation_input_tokens"] = json!(0);
            }
        }
        v
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
        // opus-5-5 有 usage_input 基准，count_tokens 不符后改比 usage（假上游为 20）。
        up.count_tokens = 55;
        let r = run(up, "claude-opus-5-5").await;
        assert!(!r.ok);
        assert_eq!(r.steps.len(), 3);
        assert_eq!(r.steps[2].0, StepId::Tokenizer);
        assert_eq!(
            r.steps[2].1,
            StepResult::Fail {
                reason: "分词结果和 Claude 不一致（得到 20，应为 68），可能不是 Claude 模型".into()
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

    fn last_fail(r: &VerifyReport) -> (StepId, String) {
        assert!(!r.ok, "{r:?}");
        match r.steps.last().unwrap() {
            (id, StepResult::Fail { reason }) => (id.clone(), reason.clone()),
            other => panic!("最后一步不是 Fail：{other:?}"),
        }
    }

    fn sig_fail(reason: &str) -> (StepId, String) {
        (StepId::Signature, reason.to_string())
    }

    #[tokio::test]
    async fn tampered_replay_400_without_signature_word_fails() {
        let mut up = FakeUpstream::new("claude-opus-5-5");
        up.tampered_reply = Some((400, "messages: invalid request"));
        let r = run(up, "claude-opus-5-5").await;
        assert_eq!(last_fail(&r), sig_fail("签名测试异常：HTTP 400"));
    }

    #[tokio::test]
    async fn tampered_replay_500_fails() {
        let mut up = FakeUpstream::new("claude-opus-5-5");
        up.tampered_reply = Some((500, "Invalid signature (internal)"));
        let r = run(up, "claude-opus-5-5").await;
        assert_eq!(last_fail(&r), sig_fail("签名测试异常：HTTP 500"));
    }

    #[tokio::test]
    async fn missing_signature_after_retry_fails() {
        let mut up = FakeUpstream::new("claude-opus-5-5");
        up.omit_signature = true;
        let r = run(up, "claude-opus-5-5").await;
        assert_eq!(last_fail(&r), sig_fail("接口不支持 Claude 的思考签名"));
    }

    fn relay(original: Option<u64>, tampered: Option<u64>) -> FakeUpstream {
        let mut up = FakeUpstream::new("claude-opus-5-5");
        up.replay_tokens = Some((original, tampered));
        up
    }

    #[tokio::test]
    async fn relay_stripping_tampered_thinking_passes() {
        // UCloud modelverse 实测：原签名回放 273，篡改回放 141（2026-11-18）。
        for (orig, tampered) in [(273, 141), (100, 84), (300, 270)] {
            let r = run(relay(Some(orig), Some(tampered)), "claude-opus-5-5").await;
            assert!(r.ok, "{orig}->{tampered}: {:?}", r.steps);
        }
    }

    #[tokio::test]
    async fn relay_with_unchanged_tokens_fails() {
        // DeepSeek 实测：篡改前后都是 108；差值不到 max(16, V/10) 也算没校验。
        for (orig, tampered) in [(108, 108), (100, 85), (300, 271), (108, 120)] {
            let r = run(relay(Some(orig), Some(tampered)), "claude-opus-5-5").await;
            assert_eq!(
                last_fail(&r),
                sig_fail("签名没有被校验，不是真正的 Claude"),
                "{orig}->{tampered}"
            );
        }
    }

    #[tokio::test]
    async fn prefix_cache_hit_is_not_a_stripped_block() {
        // 非 Claude 接口有前缀缓存时，篡改回放的 input_tokens 会因缓存命中骤降，总输入不变 → 不能放过。
        let mut up = relay(Some(273), Some(20));
        up.tampered_cache_read = Some(253);
        let r = run(up, "claude-opus-5-5").await;
        assert_eq!(last_fail(&r), sig_fail("签名没有被校验，不是真正的 Claude"));
        // 总输入确实下降（剥掉 thinking）时仍通过，即便部分来自缓存。
        let mut up = relay(Some(273), Some(41));
        up.tampered_cache_read = Some(100);
        assert!(run(up, "claude-opus-5-5").await.ok);
    }

    #[tokio::test]
    async fn malformed_cache_field_cannot_pass_by_difference() {
        // input_tokens 骤降、缓存数写成字符串 / 浮点：旧实现按 0 算会误放。
        for extra in [
            json!({"cache_read_input_tokens": "253"}),
            json!({"cache_read_input_tokens": 253.0}),
            json!({"cache_read_input_tokens": u64::MAX, "cache_creation_input_tokens": 1}),
        ] {
            let mut up = relay(Some(273), Some(20));
            up.tampered_usage_extra = Some(extra.clone());
            let r = run(up, "claude-opus-5-5").await;
            assert_eq!(last_fail(&r), sig_fail("签名没有被校验，不是真正的 Claude"), "{extra}");
        }
    }

    #[tokio::test]
    async fn tampered_replay_is_sent_before_control() {
        // 不校验签名、前缀缓存放在非标准字段：后发的回放 input_tokens 必然更少。
        // 先发篡改回放后，缓存只会让原签名回放（V）变小，差分判据不可能因此误放。
        let mut up = FakeUpstream::new("claude-opus-5-5");
        up.prefix_cache = Some((273, 200));
        let order = up.replay_order.clone();
        let r = run(up, "claude-opus-5-5").await;
        assert_eq!(*order.lock().unwrap(), [true, false]);
        assert_eq!(last_fail(&r), sig_fail("签名没有被校验，不是真正的 Claude"));
    }

    #[tokio::test]
    async fn genuine_claude_replays_tampered_first() {
        let up = FakeUpstream::new("claude-opus-5-5");
        let order = up.replay_order.clone();
        assert!(run(up, "claude-opus-5-5").await.ok);
        assert_eq!(*order.lock().unwrap(), [true, false]);
    }

    #[tokio::test]
    async fn difference_path_requires_tokenizer_pass() {
        // 无分词基准的型号：中转剥 thinking 的 2xx 回放不再放行，只认签名报错。
        let r = run(relay(Some(273), Some(141)), "claude-opus-9").await;
        assert_eq!(r.steps[2].1, StepResult::Skip { reason: "该型号没有分词基准，仅凭签名判断".into() });
        assert_eq!(last_fail(&r), sig_fail(SKIP_TOKENIZER_DIFF_FAIL));
        // 有基准且分词通过时仍放行。
        assert!(run(relay(Some(273), Some(141)), "claude-opus-5-5").await.ok);
    }

    #[tokio::test]
    async fn tiny_original_replay_cannot_pass_by_difference() {
        let r = run(relay(Some(10), Some(0)), "claude-opus-5-5").await;
        assert_eq!(last_fail(&r), sig_fail("签名没有被校验，不是真正的 Claude"));
    }

    #[test]
    fn total_input_includes_cache_fields() {
        let v = json!({"usage": {"input_tokens": 20, "cache_read_input_tokens": 253}});
        assert_eq!(total_input(&v), Some(273));
        let v = json!({"usage": {"input_tokens": 5, "cache_read_input_tokens": 7, "cache_creation_input_tokens": 9}});
        assert_eq!(total_input(&v), Some(21));
        assert_eq!(total_input(&json!({"usage": {"cache_read_input_tokens": 253}})), None);
    }

    #[test]
    fn total_input_rejects_malformed_cache_fields() {
        for bad in [
            json!({"usage": {"input_tokens": 20, "cache_read_input_tokens": 253.0}}),
            json!({"usage": {"input_tokens": 20, "cache_read_input_tokens": "253"}}),
            json!({"usage": {"input_tokens": 20, "cache_creation_input_tokens": -1}}),
            json!({"usage": {"input_tokens": u64::MAX, "cache_read_input_tokens": 1}}),
            json!({"usage": {"input_tokens": 1, "cache_read_input_tokens": u64::MAX, "cache_creation_input_tokens": 1}}),
        ] {
            assert_eq!(total_input(&bad), None, "{bad}");
        }
        // null 视同缺省
        assert_eq!(total_input(&json!({"usage": {"input_tokens": 20, "cache_read_input_tokens": null}})), Some(20));
    }

    #[tokio::test]
    async fn tampered_replay_without_usage_fails() {
        let r = run(relay(Some(273), None), "claude-opus-5-5").await;
        assert_eq!(last_fail(&r), sig_fail("签名没有被校验，不是真正的 Claude"));
    }

    #[tokio::test]
    async fn original_replay_without_usage_fails() {
        let r = run(relay(None, Some(141)), "claude-opus-5-5").await;
        assert_eq!(last_fail(&r), sig_fail("签名测试异常：原签名回放缺少用量"));
    }

    #[tokio::test]
    async fn rejecting_every_replay_fails_on_control() {
        let mut up = FakeUpstream::new("claude-opus-5-5");
        up.reject_all_replays = true;
        let r = run(up, "claude-opus-5-5").await;
        assert_eq!(last_fail(&r), sig_fail("签名测试异常：原签名回放返回 HTTP 400"));
    }

    #[tokio::test]
    async fn non_claude_json_fails_format() {
        let mut up = FakeUpstream::new("claude-opus-5-5");
        up.bad_json = true;
        let r = run(up, "claude-opus-5-5").await;
        assert_eq!(last_fail(&r), (StepId::Format, "不是 Claude 格式的接口".into()));
        assert_eq!(r.steps.len(), 2);
    }

    #[tokio::test]
    async fn wrong_sse_sequence_fails_format() {
        let mut up = FakeUpstream::new("claude-opus-5-5");
        up.bad_sse = true;
        let r = run(up, "claude-opus-5-5").await;
        assert_eq!(last_fail(&r), (StepId::Format, "不是 Claude 格式的接口".into()));
    }

    #[test]
    fn stream_check_ignores_ping_events() {
        let ev = |names: &[&str]| names.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // 中转站常在 message_start 前先发 ping 保活（UCloud modelverse 实测，2026-11-18）。
        assert!(is_claude_stream(&ev(&[
            "ping", "message_start", "content_block_start", "ping", "message_delta", "message_stop"
        ])));
        assert!(!is_claude_stream(&ev(&["ping", "content_block_start", "message_delta", "message_stop"])));
        assert!(!is_claude_stream(&ev(&["message_start", "content_block_start", "message_delta", "ping"])));
    }

    #[tokio::test]
    async fn tokenizer_falls_back_to_usage_input() {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let want = Some(Baseline { count_tokens: 68, usage_input: Some(25) });
        for (usage, expected) in [
            (25, StepResult::Pass),
            (
                30,
                fail("分词结果和 Claude 不一致（得到 30，应为 25），可能不是 Claude 模型"),
            ),
        ] {
            let mut fake = FakeUpstream::new("claude-opus-5-5");
            fake.count_tokens = 55;
            fake.usage_input = usage;
            let up = Upstream { client: &client, base: serve(fake).await, key: KEY, last: AtomicU16::new(200) };
            let got = step_tokenizer(&up, &AuthStyle::XApiKey, "claude-opus-5-5", want.clone()).await;
            assert_eq!(got, expected, "usage={usage}");
        }
    }

    #[tokio::test]
    async fn key_echoed_across_truncation_point_is_scrubbed() {
        let mut up = FakeUpstream::new("claude-opus-5-5");
        up.error_message = Some(format!("{}{KEY} rest", "x".repeat(195)));
        let r = run(up, "claude-opus-5-5").await;
        let (id, reason) = last_fail(&r);
        assert_eq!(id, StepId::Auth);
        assert!(reason.starts_with("接口返回 HTTP 400："), "{reason}");
        assert!(!reason.contains(&KEY[..10]), "{reason}");
    }

    #[tokio::test]
    async fn model_echo_requires_exact_or_date_suffix() {
        let mut up = FakeUpstream::new("claude-opus-5-5");
        up.count_tokens = 66;
        let r = run(up, "claude-opus-5").await;
        assert_eq!(
            last_fail(&r),
            (
                StepId::ModelName,
                "接口返回的模型是 claude-opus-5-5，和填写的 claude-opus-5 不一致".into()
            )
        );
    }

    #[tokio::test]
    async fn invalid_input_fails_auth_without_request() {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        // 不起假上游：一旦发请求就会连接失败，文案会不同。
        let r = verify(&client, "http://127.0.0.1:9", KEY, "qwen3-max", |_, _| {}).await;
        let (id, reason) = last_fail(&r);
        assert_eq!(id, StepId::Auth);
        assert_eq!(reason, crate::proxy::custom_api::validate_model("qwen3-max").unwrap_err());
        let r = verify(&client, "api.x.com", KEY, "claude-opus-5-5", |_, _| {}).await;
        assert_eq!(last_fail(&r), (StepId::Auth, "Base URL 必须以 http:// 或 https:// 开头".into()));
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
            let mut up = FakeUpstream::new("claude-opus-5-5");
            up.tampered_reply = Some((status, "overloaded"));
            let r = run(up, "claude-opus-5-5").await;
            assert!(!r.ok && r.transient, "{status}: {r:?}");
        }
        // 连接失败：网络类
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let r = verify(&client, "http://127.0.0.1:9", KEY, "claude-opus-5-5", |_, _| {}).await;
        assert!(!r.ok && r.transient, "{r:?}");
        // 判定类：签名没校验、格式不对、分词不符、400 非签名报错、模型名不符、认证 401、输入不合法
        let mut fake = FakeUpstream::new("claude-opus-5-5");
        fake.genuine = false;
        let mut bad_json = FakeUpstream::new("claude-opus-5-5");
        bad_json.bad_json = true;
        let mut bad_tok = FakeUpstream::new("claude-opus-5-5");
        bad_tok.count_tokens = 55;
        let mut bad_400 = FakeUpstream::new("claude-opus-5-5");
        bad_400.tampered_reply = Some((400, "messages: invalid request"));
        for up in [fake, bad_json, bad_tok, bad_400, FakeUpstream::new("claude-sonnet-5-5")] {
            let r = run(up, "claude-opus-5-5").await;
            assert!(!r.ok && !r.transient, "{r:?}");
        }
        let base = serve(FakeUpstream::new("claude-opus-5-5")).await;
        let r = verify(&client, &base, "sk-wrong-key-abcdefghijk", "claude-opus-5-5", |_, _| {}).await;
        assert!(!r.ok && !r.transient, "{r:?}");
        let r = verify(&client, "api.x.com", KEY, "claude-opus-5-5", |_, _| {}).await;
        assert!(!r.ok && !r.transient, "{r:?}");
        assert!(!run(FakeUpstream::new("claude-opus-5-5"), "claude-opus-5-5").await.transient);
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
