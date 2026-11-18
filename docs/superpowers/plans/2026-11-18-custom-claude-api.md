# 自定义 Claude API 接入 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 用户在 CC Bridge 里添加自己的 Claude API（Base URL / API Key / 模型名），通过「真 Claude」校验后作为 `api-<model>` 出现在 Claude Code 的 `/model` 里，请求换上用户 key 转发到用户地址；订阅透传与 Codex 路由行为不变。

**Architecture:** 代理侧新增 `proxy/custom_api/` 模块：进程内全局表（同 `overrides.rs` 的 RwLock 模式，客户端随时热更新）、`CustomApiProvider`（基于 Anthropic 透传，只改 `model` 字段和凭证头）、校验器（格式 → 分词指纹 → 签名篡改 → 模型名）。客户端侧把条目存到 app data 下单独的 `custom_apis.json`，经新的 Tauri 命令增删改与校验，接管时把已校验条目追加进 `modelPicker`。

**Tech Stack:** Rust（Tauri 2、axum、reqwest、tokio、serde_json）、原生 TypeScript + Vite 前端。

**Spec:** `docs/superpowers/specs/2026-11-18-custom-claude-api-design.md`

## Global Constraints

- 选单名 `api-<model>`；同一模型名多条时 `api-<model>-2`、`-3`…，按条目在列表中的顺序编号。显示名 `API <model>`，描述 `自己的 API · <base_url 主机名>`。
- 模型名只接受 `^claude-[a-z0-9-]+$`（可带 `-YYYYMMDD` 日期后缀，正则已覆盖）；其余（含 `us.anthropic.…`、`…@…`、`…:0`）保存时拒绝。
- API key 不进日志、不进请求统计、不进 `traffic` 抓包、不进 `settings.json`、不进 `StatusDto.config`；返回前端只给掩码：前 7 位 + `…` + 后 4 位，长度 ≤ 12 时只给 `…` + 后 2 位。
- 自定义条目存在 `<data_dir>/custom_apis.json`，**不放进 `AppConfig`**（前端 `save_config` 会整份回写 `AppConfig`，放进去会被清空且 key 会到前端）。（与设计文档第 1 节的「同一个文件」不同，此处为实现时的修正。）
- 订阅透传：`AnthropicProvider` 代码与现有测试不改。
- 转发给用户 API 时：删除入站 `authorization`、`x-api-key`；`anthropic-beta` 中去掉所有以 `oauth-` 开头的标志（删空则整头删除）；按 `auth_style` 加 `x-api-key: <key>` 或 `authorization: Bearer <key>`。
- 校验：单步超时 30 秒；全部通过才写 `verified_at`；任一步失败立即停止。
- 宁可误拒，不可误放。

## Review Focus

1. 用户编辑一条已校验条目改了 key 但校验失败 → 旧条目应保持原样可用，不能变成「新 key + 已校验」。（Task 4 测试 `failed_reverify_keeps_old_entry`）
2. 只配了自定义 API、没有 Codex 登录（Codex 模型列表为空）→ 接管仍应写入 `api-…` 条目。（Task 4 测试 `takeover_with_only_custom_apis`）
3. 删除条目后，`settings.json` 里 `model` 正好是被删的 `api-…` → 下次接管 / 撤销时应恢复为接管前的 model，不留悬空名。（Task 4 测试 `revoke_restores_model_when_api_entry_selected`）
4. 用户填的 Base URL 带结尾 `/` 或带 `/v1` → 应规范化为不带两者，避免拼出 `//v1` 或 `/v1/v1/messages`。（Task 1 测试 `normalize_base_url_variants`）
5. Claude Code 发来带 `[1m]` 后缀的 `api-claude-opus-5-5[1m]` → 路由仍命中该条目。（Task 2 测试 `routes_api_name_with_1m_suffix`）

---

### Task 1: 自定义 API 数据模型与全局表

**Files:**
- Create: `src-tauri/src/proxy/custom_api/mod.rs`
- Modify: `src-tauri/src/proxy/mod.rs`（加 `pub mod custom_api;`）

**Interfaces:**
- Produces:
  - `#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)] pub enum AuthStyle { XApiKey, Bearer }`（serde 小写 `x_api_key` / `bearer`）
  - `#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)] pub struct CustomApi { pub id: String, pub base_url: String, pub api_key: String, pub model: String, pub auth_style: AuthStyle, pub verified_at: Option<u64> }`
  - `pub fn validate_model(model: &str) -> Result<(), String>`
  - `pub fn normalize_base_url(raw: &str) -> Result<String, String>`（去首尾空白、去结尾 `/`、去结尾 `/v1`；必须 `http://` 或 `https://` 开头）
  - `pub fn mask_key(key: &str) -> String`
  - `pub fn picker_names(entries: &[CustomApi]) -> Vec<(String, CustomApi)>`：只含 `verified_at.is_some()` 的条目，按顺序生成 `api-<model>`、重名加 `-2`…
  - `pub fn set_entries(entries: Vec<CustomApi>)`、`pub fn lookup(picker_name: &str) -> Option<CustomApi>`（传入名先经 `registry::normalize_incoming_model` 去 `[1m]`）、`pub fn picker_list() -> Vec<String>`
  - 全局：`static ENTRIES: RwLock<Vec<(String, CustomApi)>>`，`set_entries` 内部调用 `picker_names` 后整体替换。

- [ ] **Step 1: 写失败测试**（`mod.rs` 内 `#[cfg(test)]`）

```rust
#[test] fn validate_model_accepts_official_ids() {
    for ok in ["claude-opus-5-5", "claude-haiku-4-5-20251001", "claude-fable-5-1"] { assert!(validate_model(ok).is_ok(), "{ok}"); }
    for bad in ["us.anthropic.claude-opus-5-5-v1:0", "claude-opus-5-5@20260101", "qwen3-max", "Claude-Opus", "claude-opus 5", ""] { assert!(validate_model(bad).is_err(), "{bad}"); }
}
#[test] fn normalize_base_url_variants() {
    for raw in ["https://api.x.com", "https://api.x.com/", "https://api.x.com/v1", " https://api.x.com/v1/ "] {
        assert_eq!(normalize_base_url(raw).unwrap(), "https://api.x.com");
    }
    assert_eq!(normalize_base_url("https://x.com/proxy/anthropic").unwrap(), "https://x.com/proxy/anthropic");
    assert!(normalize_base_url("api.x.com").is_err());
}
#[test] fn mask_key_hides_middle() {
    assert_eq!(mask_key("sk-ant-api03-abcdefghijklmnop1234"), "sk-ant-…1234");
    assert_eq!(mask_key("short123"), "…23");
}
#[test] fn picker_names_skip_unverified_and_number_duplicates() {
    let e = |id: &str, m: &str, v: bool| CustomApi { id: id.into(), base_url: "https://a".into(), api_key: "k".into(), model: m.into(), auth_style: AuthStyle::XApiKey, verified_at: v.then_some(1) };
    let names: Vec<String> = picker_names(&[e("1","claude-opus-5-5",true), e("2","claude-opus-5-5",false), e("3","claude-opus-5-5",true), e("4","claude-sonnet-5-5",true)]).into_iter().map(|(n,_)| n).collect();
    assert_eq!(names, ["api-claude-opus-5-5", "api-claude-opus-5-5-2", "api-claude-sonnet-5-5"]);
}
#[test] fn lookup_strips_1m_suffix() {
    set_entries(vec![CustomApi { id: "1".into(), base_url: "https://a".into(), api_key: "k".into(), model: "claude-opus-5-5".into(), auth_style: AuthStyle::XApiKey, verified_at: Some(1) }]);
    assert_eq!(lookup("api-claude-opus-5-5[1m]").unwrap().id, "1");
    assert!(lookup("claude-opus-5-5").is_none());
    set_entries(vec![]);
}
```

- [ ] **Step 2: 运行确认失败** — `cd src-tauri && cargo test custom_api::tests` → 编译失败（模块不存在）。
- [ ] **Step 3: 实现上述接口。** 全局表测试共享状态：`lookup_strips_1m_suffix` 结尾清空；如出现并发干扰，给涉及全局表的测试加同一把 `static TEST_LOCK: Mutex<()>`。
- [ ] **Step 4: 运行确认通过** — `cargo test custom_api::tests` → 5 passed。
- [ ] **Step 5: 提交** — `git add src-tauri/src/proxy/custom_api src-tauri/src/proxy/mod.rs && git commit -m "feat(custom-api): 自定义 Claude API 数据模型、校验规则与进程内路由表"`

---

### Task 2: CustomApiProvider 转发与路由接入

**Files:**
- Create: `src-tauri/src/proxy/custom_api/provider.rs`
- Modify: `src-tauri/src/proxy/custom_api/mod.rs`（`pub mod provider;`）
- Modify: `src-tauri/src/proxy/registry.rs:146-180`（`provider_for_model` 开头先查自定义表）、`registry.rs:52-60`（`Registry::new` 注册 `"custom-api"` handler）
- Modify: `src-tauri/src/proxy/server.rs:1687`（`provider.name() != "anthropic"` 改为 `!matches!(provider.name(), "anthropic" | "custom-api")`）
- Modify: `src-tauri/src/stats.rs:106`（`"anthropic"` → `"claude"`，`"custom-api"` → `"api"`，其余 `"codex"`）；`src/api.ts` 的 `Activity.route` 类型加 `'api'`

**Interfaces:**
- Consumes: Task 1 的 `lookup`、`CustomApi`、`AuthStyle`。
- Produces:
  - `pub struct CustomApiProvider { client: reqwest::Client }`，`name() == "custom-api"`，`supported_models()` 返回 `custom_api::picker_list()`。
  - `pub(crate) fn rewrite_model(raw: &[u8], real_model: &str) -> Option<Vec<u8>>`：JSON 解析后把顶层 `model` 改为 `real_model` 再序列化（项目已启用 serde_json 的保序；非 JSON 返回 None 原样转发）。在此之后再套用 `providers::anthropic::sanitize_anthropic_request`（改为 `pub(crate)`）。
  - `pub(crate) fn outbound_headers(incoming: &HeaderMap, entry: &CustomApi) -> HeaderMap`：沿用 anthropic 的 `is_stripped_request_header`（改为 `pub(crate)`），再按 Global Constraints 处理凭证与 `oauth-*` beta。
  - `pub(crate) fn build_client() -> reqwest::Client`：同 `AnthropicProvider::new` 的出口代理与禁重定向。Task 3 复用。
- 路由：`provider_for_model` 第一行：`if crate::proxy::custom_api::lookup(raw_model).is_some() { return self.handlers.get("custom-api").cloned(); }`。

- [ ] **Step 1: 写失败测试**（`provider.rs` 内；上游用 `tokio::net::TcpListener::bind("127.0.0.1:0")` + axum 起假服务器记录收到的头和体，参考 `providers/codex/chat_completions/mod.rs:317` 的 `mock_backend` 写法）

```rust
#[test] fn rewrite_model_only_touches_model() {
    let raw = br#"{"model":"api-claude-opus-5-5","max_tokens":10,"messages":[{"role":"user","content":"hi"}]}"#;
    let out: Value = serde_json::from_slice(&rewrite_model(raw, "claude-opus-5-5").unwrap()).unwrap();
    assert_eq!(out["model"], "claude-opus-5-5");
    assert_eq!(out["messages"][0]["content"], "hi");
}
#[test] fn outbound_headers_swap_credentials() {
    // incoming: authorization: Bearer sk-ant-oat01-SUB, x-api-key: x, anthropic-beta: "oauth-2025-04-20,interleaved-thinking-2025-05-14", anthropic-version: 2023-06-01
    // XApiKey 条目 → 无 authorization；x-api-key == 用户 key；anthropic-beta == "interleaved-thinking-2025-05-14"；anthropic-version 保留
    // Bearer 条目 → authorization == "Bearer <key>"；无 x-api-key
    // 只有 oauth-2025-04-20 一个 beta 时 → 无 anthropic-beta 头
}
#[tokio::test] async fn relays_to_entry_base_url_with_user_key() {
    // 假上游收到：path == "/v1/messages?beta=true"、body.model == "claude-opus-5-5"、x-api-key == 用户 key、不含订阅 token 字符串 "sk-ant-oat01-SUB"
    // 响应状态与 SSE 字节原样回到调用方
}
#[test] fn routes_api_name_with_1m_suffix() {
    // set_entries(已校验 claude-opus-5-5) 后：
    // Registry::new(AliasProvider::Anthropic).provider_for_model("api-claude-opus-5-5[1m]", None).unwrap().name() == "custom-api"
    // provider_for_model("claude-opus-5-5", None).unwrap().name() == "anthropic"
    // 未校验条目的名字 → None
}
```

- [ ] **Step 2: 运行确认失败** — `cargo test custom_api` → 编译失败。
- [ ] **Step 3: 实现 `CustomApiProvider`、`rewrite_model`、`outbound_headers`、`build_client`，接入 registry / server / stats。** `handle_messages` 与 `handle_count_tokens` 都走同一个 `relay`：按 `ctx.passthrough` 里的原始模型名 `lookup` 条目（找不到返回 400 `invalid_request_error`「自定义 API 已被删除或未通过校验」），URL = `entry.base_url + path_and_query`，响应头过滤沿用 anthropic 的 `is_stripped_response_header`（改为 `pub(crate)`）。
- [ ] **Step 4: 运行确认通过** — `cargo test` 全量 → 全部通过（含原有 anthropic、registry、stats 测试）。
- [ ] **Step 5: 提交** — `git commit -m "feat(custom-api): 自定义 API 转发与路由，订阅透传不变"`

---

### Task 3: 真 Claude 校验器

**Files:**
- Create: `src-tauri/src/proxy/custom_api/fingerprint.rs`、`src-tauri/src/proxy/custom_api/verify.rs`
- Modify: `src-tauri/src/proxy/custom_api/mod.rs`（`pub mod fingerprint; pub mod verify;`）

**Interfaces:**
- Consumes: Task 1 的 `normalize_base_url`、`validate_model`、`AuthStyle`；Task 2 的 `build_client`。
- Produces:
  - `pub const FIXED_TEXT: &str = "The quick brown fox jumps over the lazy dog. 敏捷的棕色狐狸跳过了懒狗。 0123456789 !@#$%^&*() \u{e9}\u{e8} \u{1F600}";`
  - `pub struct Baseline { pub count_tokens: u32, pub usage_input: Option<u32> }`；`pub fn baseline(model: &str) -> Option<Baseline>`：按最长前缀匹配（去掉 `-YYYYMMDD` 后缀后查表）。初始 `count_tokens` 值：haiku-4-5 / sonnet-4-6 = 60；sonnet-5 / opus-4-8 / opus-5 = 66；sonnet-5-5 / opus-5-5 / fable-5-1 = 68；opus-4-7 = 71。`usage_input` 初始全为 `None`，Task 6 用真实 key 补齐。
  - `#[derive(Serialize, Clone, Debug, PartialEq)] #[serde(rename_all = "camelCase")] pub enum StepId { Auth, Format, Tokenizer, Signature, ModelName }`
  - `#[derive(Serialize, Clone, Debug, PartialEq)] #[serde(tag = "kind", rename_all = "camelCase")] pub enum StepResult { Pass, Skip { reason: String }, Fail { reason: String } }`
  - `#[derive(Serialize, Clone, Debug)] #[serde(rename_all = "camelCase")] pub struct VerifyReport { pub steps: Vec<(StepId, StepResult)>, pub auth_style: Option<AuthStyle>, pub ok: bool }`
  - `pub async fn verify(client: &reqwest::Client, base_url: &str, api_key: &str, model: &str, on_step: impl Fn(&StepId, &StepResult)) -> VerifyReport`
- 各步判定（照设计文档第 4 节，具体值如下）：
  - **Auth**：`POST {base}/v1/messages`，`{"model", "max_tokens":16, "messages":[{"role":"user","content":"Reply with OK."}]}`，头 `anthropic-version: 2023-06-01`。先 `x-api-key`；401/403 再试 Bearer。都不是 2xx → Fail「认证失败（HTTP <code>）」；其他非 2xx → Fail「接口返回 HTTP <code>：<error.message 前 200 字>」。
  - **Format**：上一步响应满足 `type=="message"`、`id` 以 `msg_` 开头、`role=="assistant"`、`content` 为数组、`usage.input_tokens`/`output_tokens` 为整数、`stop_reason` 非空；再 `stream:true` 发同一请求，SSE 事件名序列首个为 `message_start`、包含 `content_block_start` 与 `message_delta`、末个为 `message_stop`。否则 Fail「不是 Claude 格式的接口」。
  - **Tokenizer**：`baseline(model)` 为 None → Skip「该型号没有分词基准，仅凭签名判断」。否则 `POST {base}/v1/messages/count_tokens` `{"model","messages":[{"role":"user","content":FIXED_TEXT}]}`，200 且 `input_tokens == count_tokens` → Pass；否则若 `usage_input` 为 Some，发 `/v1/messages`（`max_tokens:16`，content=FIXED_TEXT）比 `usage.input_tokens`，相等 → Pass；都不等 → Fail「分词结果和 Claude 不一致（得到 <n>，应为 <m>），可能不是 Claude 模型」。
  - **Signature**：thinking 参数：模型名以 `claude-haiku-4-5`、`claude-sonnet-4-`、`claude-opus-4-` 开头用 `{"type":"enabled","budget_tokens":1024}`、`max_tokens: 2048`；其余用 `{"type":"adaptive","display":"omitted"}` + `"output_config":{"effort":"high"}`、`max_tokens: 4096`。题目：`"A train leaves at 9:47 and arrives at 13:12. How many minutes is the trip? Reason carefully."`；拿不到带非空 `signature` 的 `thinking` / `redacted_thinking` 块时换题 `"Is 391 prime? Check divisors step by step."` 重试一次，仍无 → Fail「接口不支持 Claude 的思考签名」。拿到后把签名倒数第 8 个字符换成另一个字符（是 `A` 换 `B`，否则换 `A`），历史 = 原题 + assistant[该块（改签名）+ 原 text 块] + user「Now add 1.」回放：返回 400 且 `error.message` 含 `signature`（不区分大小写）→ Pass；返回 2xx → Fail「签名没有被校验，不是真正的 Claude」；其他 → Fail「签名测试异常：HTTP <code>」。
  - **ModelName**：Auth 步响应的 `model` 等于填写名，或以 `<填写名>-` 开头 → Pass；否则 Fail「接口返回的模型是 <x>，和填写的 <y> 不一致」。
  - 任一 Fail 后不再执行后续步，`ok = false`。

- [ ] **Step 1: 写失败测试**（`verify.rs`；一个可配置的 axum 假上游 `FakeUpstream { genuine: bool, count_tokens: u32, model_echo: String, require_bearer: bool }`：genuine 时对签名被改的回放返回 400 `{"type":"error","error":{"type":"invalid_request_error","message":"messages.1.content.0: Invalid \`signature\` in \`thinking\` block"}}`，否则正常回答）

```rust
#[tokio::test] async fn genuine_claude_passes_all_steps()      // opus-5-5，count_tokens=68 → ok, 5 个 Pass, auth_style == XApiKey
#[tokio::test] async fn compatible_api_fails_on_signature()    // genuine=false，count_tokens=68 → steps[3] == Signature Fail「签名没有被校验…」, ok=false
#[tokio::test] async fn wrong_tokenizer_fails_before_signature() // count_tokens=55 且无 usage 基准 → Tokenizer Fail，steps.len()==3
#[tokio::test] async fn unknown_model_skips_tokenizer()        // model="claude-opus-9" → Tokenizer Skip，其余 Pass，ok=true
#[tokio::test] async fn bearer_fallback_detected()             // require_bearer=true → auth_style == Bearer
#[tokio::test] async fn model_echo_mismatch_fails()            // model_echo="claude-sonnet-5-5"，填 opus-5-5 → ModelName Fail
#[tokio::test] async fn dated_echo_is_accepted()               // 填 claude-haiku-4-5，回显 claude-haiku-4-5-20251001 → Pass
#[test] fn baseline_strips_date_suffix()                       // baseline("claude-haiku-4-5-20251001").unwrap().count_tokens == 60；baseline("claude-opus-9") is None
```

- [ ] **Step 2: 运行确认失败** — `cargo test custom_api::verify` → 编译失败。
- [ ] **Step 3: 实现 `fingerprint.rs` 与 `verify.rs`。** 每个请求 `.timeout(Duration::from_secs(30))`；任何错误信息里不得包含 api_key（构造错误文案时不用 reqwest 的完整 Debug 输出）。
- [ ] **Step 4: 运行确认通过** — `cargo test custom_api` → 全部通过。
- [ ] **Step 5: 提交** — `git commit -m "feat(custom-api): 真 Claude 校验（格式、分词指纹、签名篡改、模型名）"`

---

### Task 4: 客户端存储、命令与接管

**Files:**
- Create: `src-tauri/src/custom_apis.rs`（客户端侧存储与掩码 DTO）
- Modify: `src-tauri/src/lib.rs`（注册命令）、`src-tauri/src/controller.rs`（加载 / 推送到代理 / 参与接管）、`src-tauri/src/claude_settings.rs:62-64`（`model_entry` 区分 API 条目）

**Interfaces:**
- Consumes: Task 1 全部；Task 3 的 `verify`、`VerifyReport`、`StepId`、`StepResult`；Task 2 的 `build_client`。
- Produces:
  - `custom_apis.rs`：`pub fn load(data_dir: &Path) -> Vec<CustomApi>`、`pub fn save(data_dir: &Path, v: &[CustomApi]) -> io::Result<()>`（文件 `custom_apis.json`，用 `config::save_json`）、`#[derive(Serialize)] #[serde(rename_all="camelCase")] pub struct CustomApiDto { pub id: String, pub base_url: String, pub key_mask: String, pub model: String, pub picker_name: Option<String>, pub verified_at: Option<u64> }`、`pub fn to_dtos(v: &[CustomApi]) -> Vec<CustomApiDto>`。
  - `#[derive(Deserialize)] #[serde(rename_all="camelCase")] pub struct CustomApiInput { pub id: Option<String>, pub base_url: String, pub api_key: Option<String>, pub model: String }`（`api_key` 为 None 或空串 = 编辑时不改 key；新增时必填）
  - Tauri 命令：
    - `async fn save_custom_api(ctl, input: CustomApiInput) -> Result<VerifyReport, String>`：校验输入 → 合成候选条目 → `verify`（每步通过 `app.emit("custom-api-step", {id, step, result})` 推给前端）→ `ok` 时写入 `verified_at`（unix 秒）与 `auth_style` 并保存；失败时：新增 → 也保存为未通过条目（`verified_at: None`）以便用户改了再试；编辑已存在条目 → **不改动原条目**，只返回报告。
    - `async fn reverify_custom_api(ctl, id: String) -> Result<VerifyReport, String>`：用已存的条目重跑；失败则把该条 `verified_at` 置 None。
    - `fn delete_custom_api(ctl, id: String)`
  - `StatusDto` 新增 `pub custom_apis: Vec<CustomApiDto>`（前端 `Status.customApis`）。
  - Controller：`Controller::new` 里 `load` 后调用 `proxy::custom_api::set_entries`；每次增删改后 `set_entries` + `notify()`。`reconcile` 的 `models` = `effective_models(available)` 后追加 `custom_api::picker_list()`；`want` 条件中 `!available.is_empty()` 改为 `!(available.is_empty() && picker_list().is_empty())`。
  - `claude_settings::model_entry(id)`：`id` 以 `api-` 开头时返回 `{"model": id, "label": "API <真实模型名>", "description": "自己的 API · <主机名>"}`（从 `custom_api::lookup(id)` 取；查不到时 label 用 `API <id 去掉 api- 前缀>`、description `自己的 API`），否则保持原 Codex 条目。

- [ ] **Step 1: 写失败测试**

```rust
// claude_settings.rs
#[test] fn api_entry_label_and_description()        // set_entries(已校验 opus-5-5 @ https://relay.example.com) → model_entry("api-claude-opus-5-5") == {"model":"api-claude-opus-5-5","label":"API claude-opus-5-5","description":"自己的 API · relay.example.com"}
#[test] fn revoke_restores_model_when_api_entry_selected() // apply_takeover 写入 ["gpt-6.2-sol","api-claude-opus-5-5"]，doc.model 设为 "api-claude-opus-5-5"，prev_model=Some("opus") → revoke 后 model == "opus"，modelPicker 不再含 api 条目
// custom_apis.rs
#[test] fn dto_never_contains_key()                 // to_dtos 序列化成字符串后不包含完整 api_key，key_mask == mask_key(key)
#[test] fn load_save_roundtrip()                    // tempdir 写入两条再读回相等
// controller 中把「编辑失败不覆盖」「仅自定义 API 也接管」抽成纯函数测试：
#[test] fn failed_reverify_keeps_old_entry()        // pub(crate) fn merge_edit(existing: &[CustomApi], candidate: CustomApi, ok: bool) -> Vec<CustomApi>：ok=false 且 id 已存在 → 返回值等于 existing
#[test] fn takeover_with_only_custom_apis()         // pub(crate) fn takeover_models(selected_codex: Vec<String>, custom: Vec<String>) -> Vec<String>；以及 pub(crate) fn want_takeover(onboarded, takeover, running, codex_empty, custom_empty) -> bool：codex 为空、custom 非空时为 true
```

- [ ] **Step 2: 运行确认失败** — `cargo test` → 编译失败。
- [ ] **Step 3: 实现 `custom_apis.rs`、三个命令、`StatusDto.custom_apis`、`merge_edit` / `takeover_models` / `want_takeover`、`model_entry` 分支。** 新条目 `id` 用 `uuid::Uuid::new_v4().to_string()`（`Cargo.toml` 无 uuid 时改用 `format!("{:x}", 纳秒时间戳)`，不为此新增依赖）。
- [ ] **Step 4: 运行确认通过** — `cargo test` 全量通过；`cargo clippy --all-targets -- -D warnings` 无告警。
- [ ] **Step 5: 提交** — `git commit -m "feat(custom-api): 客户端存储、增删改与校验命令，接管写入 API 模型"`

---

### Task 5: 界面

**Files:**
- Modify: `src/api.ts`（类型）、`src/main.ts`（`modelsTab` 末尾加一节 + 弹窗 + 事件）、`src/styles.css`、`src/locales/*.ts`（10 个语言文件）、`src/mock.ts`（mock 数据加两条自定义 API，一条已校验一条未通过）

**Interfaces:**
- Consumes: Task 4 的 `save_custom_api` / `reverify_custom_api` / `delete_custom_api` 命令、`custom-api-step` 事件、`Status.customApis`。
- Produces（前端类型）：
  - `export interface CustomApiDto { id: string; baseUrl: string; keyMask: string; model: string; pickerName: string | null; verifiedAt: number | null }`
  - `export type StepId = 'auth' | 'format' | 'tokenizer' | 'signature' | 'modelName'`
  - `export type StepResult = { kind: 'pass' } | { kind: 'skip'; reason: string } | { kind: 'fail'; reason: string }`
  - `export interface VerifyReport { steps: [StepId, StepResult][]; authStyle: 'x_api_key' | 'bearer' | null; ok: boolean }`

- [ ] **Step 1: 实现列表。** `modelsTab` 的 `foot-note` 之前加 `<section class="capi">`：标题 `t('capiTitle')` + 按钮 `data-act="capi-add"`；每行 `API <model>`、主机名、状态徽标（已校验 ✓ / 未通过 ✗）、`data-act="capi-edit"` / `capi-verify` / `capi-del`（带 `data-id`）。删除先 `confirm(t('capiDelConfirm'))`。
- [ ] **Step 2: 实现弹窗。** 沿用更新弹窗的 `.mask` + `.sheet` 结构：三个输入 `#capi-url`、`#capi-key`（`type="password"`，编辑时 placeholder 为 `t('capiKeyKeep', { mask })`）、`#capi-model`（placeholder `claude-opus-5-5`）；按钮 `data-act="capi-save"`。点击后禁用按钮，列出 5 步（`t('capiStep_auth')` 等），监听 `custom-api-step` 逐行更新 ✓ / ✗ / –（跳过）并显示原因；结束后 `ok` 则 toast `t('capiOk')` 并关闭，失败保留输入。
- [ ] **Step 3: 文案。** 在 10 个 locale 文件加键：`capiTitle`、`capiAdd`、`capiUrl`、`capiKey`、`capiKeyKeep`、`capiModel`、`capiSave`、`capiVerify`、`capiDel`、`capiDelConfirm`、`capiVerified`、`capiFailed`、`capiOk`、`capiStep_auth`、`capiStep_format`、`capiStep_tokenizer`、`capiStep_signature`、`capiStep_modelName`、`capiHint`（中文：「只能接入真正的 Claude。Qwen、DeepSeek 等「Claude 兼容接口」会在校验时被拒绝。」）。中文与英文写准，其余语言按英文翻译。后端返回的失败原因为中文，非中文界面原样显示即可（本期不做后端文案多语言）。
- [ ] **Step 4: 验证。** `npm run build` 无类型错误；`npm run dev` 打开 `http://localhost:1420/?mock=<现有场景名>`，用 Edge 无头截图「模型」页和弹窗（`msedge --headless=new --screenshot=...`），用看图工具检查列表、徽标、弹窗与 5 步进度显示正常、中文不截断。
- [ ] **Step 5: 提交** — `git commit -m "feat(custom-api): 模型页新增「自己的 Claude API」与校验弹窗"`

---

### Task 6: 真实 key 实测、补齐指纹表、端到端验证

**Files:**
- Create: `scripts/custom-api-probe.py`（只读环境变量 `ANTHROPIC_TEST_KEY`，不打印 key）
- Modify: `src-tauri/src/proxy/custom_api/fingerprint.rs`（填 `usage_input`，必要时修正 `count_tokens`）

**Interfaces:**
- Consumes: Task 3 的固定文本与请求形状；Task 4/5 的完整功能。

- [ ] **Step 1: 跑探测脚本。** 由用户提供的真实 Anthropic API key 经环境变量传入（不写进命令行、文件、提交）。对 `claude-haiku-4-5`、`claude-sonnet-5-5`、`claude-opus-5-5`、`claude-fable-5-1` 各测：`count_tokens(FIXED_TEXT)`、`/v1/messages` 的 `usage.input_tokens`、Task 3 的签名请求形状能否拿到签名、篡改回放是否 400、原样回放是否 200。期望：每个型号「篡改 400 / 原样 200」。
- [ ] **Step 2: 把实测两列基准写进 `fingerprint.rs`**，与订阅实测的 `count_tokens` 不一致时以 API key 实测为准并在注释写明日期。`cargo test custom_api` 通过。
- [ ] **Step 3: 端到端（`npm run tauri dev`，端口 18766、沙盒 settings）。** 用真实 key 添加 `claude-haiku-4-5` → 5 步全过 → 沙盒 `settings.json` 的 `modelPicker` 出现 `api-claude-haiku-4-5`；`claude --settings <沙盒 settings 路径> -p --model api-claude-haiku-4-5 "say ok"` 正常返回；「动态」页该请求路由显示 API。再用一个 Anthropic 兼容的非 Claude 接口（如 DeepSeek `https://api.deepseek.com/anthropic` + `deepseek-flash`，模型名填 `claude-opus-5-5` 冒充）添加 → 校验失败、不进选单。订阅模型 `claude -p --model claude-haiku-4-5` 经代理仍正常。
- [ ] **Step 4: 检查 key 未泄露。** 在 app data 目录的日志、`traffic` 抓包目录、沙盒 `settings.json` 中搜索 key 后 8 位，期望 0 命中（只有 `custom_apis.json` 有）。
- [ ] **Step 5: 提交** — `git commit -m "feat(custom-api): 用真实 API 补齐分词基准，附探测脚本"`
