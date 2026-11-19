# 用量统计 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 「动态」页新增默认的「统计」分段：按天 / 月看经过 CC Bridge 的请求的等价 API 花费、请求数、Tokens、缓存命中率，可按线路 / 模型筛选、导出 CSV；原「近 7 天 N 次」「今天」不变。

**Architecture:** 代理在 `server.rs::monitor_response_body` 统一解析下游 Anthropic 协议的 usage（新 `proxy/usage_tap.rs`），经新监控事件 `UsageFinal` 写进 `CompletedRequest.usage`；`stats.rs` 按「天 × 线路 × 模型」汇总进新文件 `usage.json`（400 天，一次性导入旧 `stats.json`）；新 `pricing.rs` 内置价格表，`usage_report` / `export_usage_csv` 两个 Tauri 命令在 Rust 端聚合与算钱；前端 `main.ts` 渲染统计页，`src/usage-chart.ts` 持有常驻 ECharts 实例，整页 innerHTML 重绘后把宿主节点搬回占位处。

**Tech Stack:** Rust（Tauri 2、axum、serde_json、jiff、dirs）、原生 TypeScript + Vite、ECharts 5（按需引入）。

**Spec:** `docs/superpowers/specs/2026-11-19-usage-stats-design.md`

## Global Constraints

- 只统计 `EndpointKind::Messages`、`model.is_some()` 的已完成 / 失败请求（与现有 `ingest` 过滤一致）。
- 线路标识沿用 `stats.rs::to_activity`：`anthropic`→`"claude"`、`custom-api`→`"api"`、其余→`"codex"`。界面文案：`claude`=「Claude 订阅」（已有 `routeClaude`）、`codex`=「GPT（Codex 订阅）」（新 `routeGpt`）、`api`=「自己的 API」（已有 `routeApi`）。
- 四项 token：未命中输入 `input`、缓存读 `cache_read`、缓存写 `cache_write_5m` + `cache_write_1h`、输出 `output`。Anthropic `input_tokens` 不含缓存直接用；无 `cache_creation` 细分时 `cache_creation_input_tokens` 全记 5m。SSE 每个字段**后到覆盖先到**，不取最大值。
- 汇总文件 `<data_dir>/usage.json`，`{"version":2,"days":{日期:{线路:{模型:{"n","e","ms","in","cr","cw","cw1h","out"}}}}}`，全部数字字段 `#[serde(default)]`；保留 `KEEP_DAYS = 400`（删除 `日期 <= 今天 − 400 天`）；写 `usage.json.tmp` 再 rename。`stats.json` 只读一次用于导入，不写不删。
- 价格（美元/百万 token：输入/输出/缓存读/缓存写5m/缓存写1h）：
  `claude-opus-5-5` 4/20/0.2/5/8；`claude-sonnet-5-5` 2/10/0.1/2.5/4；`claude-sonnet-5` 2/10/0.2/2.5/4；`claude-fable-5-2` 8/40/0.2/10/16；`claude-fable-5-1` 10/50/0.25/12.5/20；`claude-opus-5` 5/25/0.5/6.25/10；`claude-haiku-5-5` 0.1/0.5/0.01/0.125/0.2（只取 ≤100K 档）；`claude-haiku-4-5` 1/5/0.1/1.25/2；`claude-opus-4-8`、`claude-opus-4-7` 5/25/0.5/6.25/10；`claude-sonnet-4-6` 3/15/0.3/3.75/6。
- 模型匹配：小写 → 去 `[...]` 后缀 → 去 `-YYYYMMDD` / `@YYYYMMDD` → 与键完全相等；不做最长前缀。只有 `claude` 线路计价，`codex`、`api` 一律未知价格。
- 未知价格提示文案（zh）：「含未知价格模型：{models}。这些模型的 token 已统计，但不计入金额。」金额标签「等价 API 价」。
- 线路配色：`claude` `#D97757`、`codex` `#10A37F`、`api` `#6E59D9`，全图一致、深浅色相同。
- ECharts：只引 `echarts/core`、`BarChart`、`GridComponent`、`CanvasRenderer`；`tooltip: { show: false }`；图表宿主常驻，不随重绘 `dispose`。
- 缓存命中率 = 缓存读 ÷（未命中输入 + 缓存读 + 缓存写），分母 0 显示「—」。日均 = 合计 ÷ 范围天数（含空天，截至今天）。
- 新增文案先加 `src/locales/zh.ts`，其余 9 种语言（en、zh-TW、ja、ko、es、fr、de、ru、pt）同一任务补齐（`Dict` 类型会强制）。
- 代码注释一律中文；提交信息中文，**不带任何 `Co-Authored-By`**。
- 应用可能正以 `tauri dev` 运行并锁住 `src-tauri/target/debug/cc-bridge.exe`：Rust 测试一律加 `CARGO_TARGET_DIR` 指到别处，例如 Git Bash 下
  `cd src-tauri && CARGO_TARGET_DIR=../target-usage cargo test <过滤>`（PowerShell：`$env:CARGO_TARGET_DIR='..\target-usage'; cargo test <过滤>`）。不启动、不关闭正在运行的应用。`target-usage/` 不提交（确认 `.gitignore` 覆盖，否则加上）。

## Review Focus

1. **SSE 的 usage 行被拆在两个 frame 里**（`data: {"type":"message_delta",...` 半行在前一块）→ 必须拼接后再解析，不能丢。（Task 1 测试 `tap_parses_usage_split_across_chunks`）
2. **Codex 线路 `message_start` 带估算 input（偏大），`message_delta` 带真实 input（偏小）** → 终值必须是后者；监控旧字段 `update_token_count` 取最大值的逻辑不能被沿用。（Task 1 测试 `tap_later_value_overrides_estimate`；Task 2 测试 `usage_final_reaches_completed_request`）
3. **型号后缀与近似名**：`claude-sonnet-5-5-20261001` 应是 Sonnet 5.5 而不是 Sonnet 5；`claude-opus-5-5[1m]` 是 Opus 5.5；`claude-sonnet-5-6` 未知；`api` 线路即使底层是 Claude 型号也未知。（Task 1 测试 `price_lookup_normalizes_and_matches_exactly`、Task 4 测试 `api_and_codex_routes_are_unpriced`）
4. **升级时只有旧 `stats.json`**：导入后「今天」「近 7 天」数字与升级前一致，`stats.json` 原样保留，第二次启动不重复导入。（Task 3 测试 `imports_legacy_stats_json_once`）
5. **时间范围边界**：`from > to`、超过 400 天、跨月的每月桶、含空天的日均。（Task 4 测试 `report_clamps_range_and_buckets_by_month`）

---

### Task 1: usage 解析器与价格表（纯函数）

**Files:**
- Create: `src-tauri/src/proxy/usage_tap.rs`（`TokenUsage`、`UsageTap`）
- Create: `src-tauri/src/pricing.rs`（价格表、`normalize_model`、`price_for`、`cost_usd`）
- Modify: `src-tauri/src/proxy/mod.rs`（`pub mod usage_tap;`）、`src-tauri/src/lib.rs`（`pub mod pricing;`）
- Test: 两个文件内 `#[cfg(test)] mod tests`

**Interfaces:**
- Produces:
  - `#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)] pub struct TokenUsage { pub input: u64, pub cache_read: u64, pub cache_write_5m: u64, pub cache_write_1h: u64, pub output: u64 }`，`impl TokenUsage { pub fn total(&self) -> u64; pub fn add(&mut self, o: &TokenUsage); pub fn is_empty(&self) -> bool }`
  - `pub struct UsageTap`；`impl UsageTap { pub fn new(content_type: Option<&str>) -> Self; pub fn feed(&mut self, chunk: &[u8]); pub fn finish(self) -> Option<TokenUsage> }`（`text/event-stream` 按行；其它累积至 2 MiB 后结尾按 JSON 读顶层 `usage`；行缓冲上限 1 MiB；只解析以 `data:` 开头且含 `"usage"` 的行；从未见到 usage 返回 `None`）
  - `pub struct Price { pub input: f64, pub output: f64, pub cache_read: f64, pub cache_write_5m: f64, pub cache_write_1h: f64 }`
  - `pub fn normalize_model(model: &str) -> String`
  - `pub fn price_for(route: &str, model: &str) -> Option<&'static Price>`（`route != "claude"` 恒 `None`）
  - `pub fn cost_usd(u: &TokenUsage, p: &Price) -> f64`

- [ ] **Step 1: 写失败测试**（`usage_tap.rs`）
  - `tap_reads_anthropic_stream`：喂 `message_start`（usage `input_tokens:12, cache_read_input_tokens:3000, cache_creation_input_tokens:200`）+ `message_delta`（`output_tokens:55`）→ `finish() == Some(TokenUsage{input:12,cache_read:3000,cache_write_5m:200,cache_write_1h:0,output:55})`
  - `tap_splits_cache_creation_ttl`：usage 带 `cache_creation:{ephemeral_5m_input_tokens:100, ephemeral_1h_input_tokens:40}` 与 `cache_creation_input_tokens:140` → `cache_write_5m==100 && cache_write_1h==40`
  - `tap_parses_usage_split_across_chunks`：把上面的 SSE 字节在 `message_delta` 的 JSON 中间切成两块 `feed` → 结果与不切相同
  - `tap_later_value_overrides_estimate`：`message_start` `input_tokens:9000`，`message_delta` `input_tokens:120, cache_read_input_tokens:8800, output_tokens:30` → `input==120 && cache_read==8800`
  - `tap_reads_non_stream_json`：`content_type=Some("application/json")`，body 为带 `usage` 的 message JSON，分三块喂 → 正确四项
  - `tap_without_usage_is_none`：错误 JSON `{"type":"error",...}` → `None`
- [ ] **Step 2: 写失败测试**（`pricing.rs`）
  - `price_lookup_normalizes_and_matches_exactly`：`price_for("claude","claude-sonnet-5-5-20261001").unwrap().input == 2.0 && .cache_read == 0.1`；`price_for("claude","claude-opus-5-5[1m]").unwrap().output == 20.0`；`price_for("claude","Claude-Haiku-4-5@20251001").is_some()`；`price_for("claude","claude-sonnet-5-6").is_none()`；`price_for("api","claude-opus-5-5").is_none()`；`price_for("codex","gpt-6.1-sol").is_none()`
  - `cost_counts_each_bucket`：Opus 5.5，`TokenUsage{input:1_000_000,cache_read:1_000_000,cache_write_5m:1_000_000,cache_write_1h:1_000_000,output:1_000_000}` → `(cost_usd(..) - (4.0+0.2+5.0+8.0+20.0)).abs() < 1e-9`
  - `price_table_matches_spec`：逐行断言 11 个键的五个价格与 Global Constraints 一致
- [ ] **Step 3: 运行确认失败**：`cd src-tauri && CARGO_TARGET_DIR=../target-usage cargo test --lib usage_tap pricing`（编译失败 / 断言失败均可）
- [ ] **Step 4: 实现**上述签名；价格表用 `static PRICES: &[(&str, Price)]`。
- [ ] **Step 5: 运行通过**：同 Step 3 命令全部 PASS
- [ ] **Step 6: 提交**：`git add src-tauri/src/proxy/usage_tap.rs src-tauri/src/pricing.rs src-tauri/src/proxy/mod.rs src-tauri/src/lib.rs && git commit -m "feat(usage): 新增 usage 增量解析器与 Claude 价格表"`

---

### Task 2: 监控接入四项 token

**Files:**
- Modify: `src-tauri/src/proxy/monitor.rs`（新事件、字段、`finish_active` 复制 `usage`）
- Modify: `src-tauri/src/proxy/server.rs`（`monitor_response_body` 喂 tap，结束时上报）
- Modify: `src-tauri/src/proxy/monitor/mock.rs`、`snapshot.rs`、`remote.rs`（如有构造 `ActiveRequest`/`CompletedRequest` 的地方补 `usage: None`）
- Test: `monitor.rs` 测试模块、`server.rs` 的 `request_id_header_tests` 旁新增 `usage_tap_tests`

**Interfaces:**
- Consumes: `proxy::usage_tap::{TokenUsage, UsageTap}`
- Produces:
  - `MonitorEvent::UsageFinal { request_id: String, usage: TokenUsage }`
  - `impl MonitorHandle { pub fn usage_final(&self, request_id: impl Into<String>, usage: TokenUsage) }`
  - `ActiveRequest.usage: Option<TokenUsage>`、`CompletedRequest.usage: Option<TokenUsage>`（`pub`）
  - `RequestMonitorGuard` 增加 `fn usage(&mut self, u: TokenUsage)`（monitor 还在时调用 `usage_final`）

- [ ] **Step 1: 写失败测试**
  - `monitor.rs::usage_final_reaches_completed_request`：`request_started` → `stream_progress(.., Some(9000), Some(0))` → `usage_final("r1", TokenUsage{input:120,cache_read:8800,..,output:30})` → `request_completed("r1",200,None,None)` → `snapshot().recent[0].usage == Some(..120..)`，且旧字段 `input_tokens` 仍为 `Some(9000)`（不改旧行为）
  - `server.rs::usage_tap_tests::reports_usage_before_completion`：用 `MonitorHandle::new(10)` + `RequestMonitorGuard::new(Some(m.clone()), "u1")`，`text/event-stream` 响应 body 由 `futures_util::stream::iter` 的三个 `Bytes` 组成（usage 行跨块），`monitor_response_body` 后把 body 读完 → `m.snapshot().recent[0].usage.unwrap().cache_read == 8800` 且 `status == Completed`
  - `server.rs::usage_tap_tests::error_response_has_no_usage`：500 JSON 错误体 → `recent[0].usage.is_none()`
- [ ] **Step 2: 运行确认失败**：`cd src-tauri && CARGO_TARGET_DIR=../target-usage cargo test --lib usage_final usage_tap_tests`
- [ ] **Step 3: 实现**：`monitor_response_body` 从 `parts.headers` 取 `content-type` 建 `UsageTap`，放进 `unfold` 状态；`Some(Ok(frame))` 时若 `frame.data_ref()` 有数据就 `feed`；`None` 与 `Some(Err)` 分支先 `if let Some(u) = tap.finish() { guard.usage(u) }` 再 `completed/failed`。`UsageFinal` 只更新 active（已完成的也兼容写入 recent 中同 id 的条目）。
- [ ] **Step 4: 运行通过**：同 Step 2；再跑 `cd src-tauri && CARGO_TARGET_DIR=../target-usage cargo test --lib proxy::` 确认原有监控 / server 测试不回归
- [ ] **Step 5: 提交**：`git commit -am "feat(usage): 代理在响应结束时上报四项 token 到请求监控"`（新文件已在 Task 1 提交）

---

### Task 3: 按天 × 线路 × 模型汇总落盘与旧文件导入

**Files:**
- Modify: `src-tauri/src/stats.rs`
- Modify: `src-tauri/src/lib.rs`（`Stats::load(data_dir.join("usage.json"), data_dir.join("stats.json"))`）
- Test: `stats.rs` 测试模块（现有 5 个测试改为新加载签名后保持语义通过）

**Interfaces:**
- Consumes: `CompletedRequest.usage`、`TokenUsage`
- Produces:
  - `#[derive(Default, Clone, Serialize, Deserialize, PartialEq, Debug)] pub struct Agg { #[serde(default)] n: u64, e: u64, ms: u64, #[serde(rename="in")] input: u64, cr: u64, cw: u64, cw1h: u64, out: u64 }`（全部 `#[serde(default)]`），`impl Agg { pub fn add_request(&mut self, ok: bool, ms: u64, u: &TokenUsage); pub fn merge(&mut self, o: &Agg); pub fn usage(&self) -> TokenUsage }`
  - `struct Persist { version: u32, days: BTreeMap<String, BTreeMap<String, BTreeMap<String, Agg>>> }`
  - `pub fn Stats::load(path: PathBuf, legacy: PathBuf) -> Self`
  - `Activity` 新增 `pub cache_read_tokens: Option<u64>, pub cache_write_tokens: Option<u64>`（来自 `usage`；`input_tokens` 有 `usage` 时改用 `usage.input`，否则沿用监控旧值）
  - `const KEEP_DAYS: i64 = 400;`
  - `fn legacy_route(model: &str) -> &'static str`（`api-`→`api`，`claude-`→`claude`，其余 `codex`）

- [ ] **Step 1: 写失败测试**
  - `records_tokens_per_route_and_model`：监控里两条 `claude-opus-5-5`（anthropic，`usage_final` 各 `cache_read:100`）、一条 `gpt-6.1-sol`（codex）→ `persist.days[今天]["claude"]["claude-opus-5-5"].cr == 200 && .n == 2`，`["codex"]["gpt-6.1-sol"].n == 1`
  - `imports_legacy_stats_json_once`：临时目录写旧格式 `stats.json`（今天 `gpt-6.2-sol` count 3 errors 1、`api-claude-opus-5-5` count 2、6 天前 `claude-opus-5-5` count 4）→ `load` 后 `overview().today.count == 5 && errors == 1`，`model_use["claude-opus-5-5"] == 4`，路由分别为 `codex/api/claude`；`save()` 后 `usage.json` 存在、`stats.json` 字节与原来完全相同；再次 `load` 后 `today.count` 仍为 5（不重复导入）
  - `prunes_days_older_than_400`：塞入 `今天−400` 与 `今天−399` 两天，ingest 一条新请求后前者被删、后者保留
  - `corrupt_usage_json_falls_back_to_empty`：`usage.json` 写入 `"not json"` → `load` 不 panic，`today.count == 0`，且**不**导入 `stats.json`（避免覆盖坏文件前丢数据：坏文件先改名为 `usage.json.bad` 再从空开始）
  - 现有 `persists_daily_counts_and_prunes_old_days` 的「超过 8 天删掉」断言改为「超过 400 天删掉」
- [ ] **Step 2: 运行确认失败**：`cd src-tauri && CARGO_TARGET_DIR=../target-usage cargo test --lib stats::`
- [ ] **Step 3: 实现**：`ingest` 写三级 map；`overview_at` 跨线路求和；`save` 仍是 `.tmp` + rename，文件名改为 `usage.json`。
- [ ] **Step 4: 运行通过**：同 Step 2 全 PASS
- [ ] **Step 5: 提交**：`git commit -am "feat(usage): 用量按天×线路×模型汇总进 usage.json，保留 400 天并导入旧 stats.json"`

---

### Task 4: 报表聚合、金额与导出命令

**Files:**
- Modify: `src-tauri/src/stats.rs`（`report`、`export_csv`）
- Modify: `src-tauri/src/lib.rs`（命令 `usage_report`、`export_usage_csv` 并注册进 `generate_handler!`）
- Test: `stats.rs` 测试模块

**Interfaces:**
- Consumes: `pricing::{price_for, cost_usd}`、`Agg`
- Produces:
  - `pub struct ReportQuery { pub from: String, pub to: String, pub granularity: Granularity, pub route: Option<String>, pub model: Option<String> }`，`pub enum Granularity { Day, Month }`（`"day"|"month"` 解析，其它返回 `Err`）
  - `#[serde(rename_all="camelCase")] pub struct Totals { count, errors, ms, input, cache_read, cache_write, output: u64, cost_usd: f64 }`
  - `pub struct ModelRow { route: String, model: String, totals: Totals, priced: bool }`
  - `pub struct Bucket { key: String, by_route: BTreeMap<String, Totals>, models: Vec<ModelRow> }`（`models` 按 `cost_usd` 降序、再按 token 降序）
  - `pub struct UsageReport { from, to: String, granularity: String, days: u32, totals: Totals, by_route: BTreeMap<String, Totals>, buckets: Vec<Bucket>, unpriced_models: Vec<String>, routes: Vec<String>, models: Vec<String> }`
  - `impl Stats { pub fn report(&self, q: &ReportQuery) -> UsageReport; fn report_at(&self, q: &ReportQuery, today: &str) -> UsageReport; pub fn export_csv(&self, q: &ReportQuery, dir: &Path) -> std::io::Result<PathBuf> }`
  - `#[tauri::command] fn usage_report(st: State<'_, Arc<stats::Stats>>, from: String, to: String, granularity: String, route: Option<String>, model: Option<String>) -> Result<stats::UsageReport, String>`
  - `#[tauri::command] fn export_usage_csv(st: State<'_, Arc<stats::Stats>>, app: AppHandle, from: String, to: String, granularity: String, route: Option<String>, model: Option<String>) -> Result<String, String>`（目录 `dirs::download_dir()`，取不到用 app data dir）

- [ ] **Step 1: 写失败测试**
  - `report_sums_cost_for_priced_models`：今天 `claude/claude-opus-5-5` `in=1_000_000,out=1_000_000` → `totals.cost_usd == 24.0`，`by_route["claude"].cost_usd == 24.0`，`unpriced_models` 为空
  - `api_and_codex_routes_are_unpriced`：再加 `api/api-claude-opus-5-5`（自己的 API 的模型名就是选单名 `api-…`）与 `codex/gpt-6.1-sol` → 总 `cost_usd` 仍为 24.0，`totals.input` 含三者之和，`unpriced_models == ["api-claude-opus-5-5", "gpt-6.1-sol"]`（去重、按字典序）
  - `report_clamps_range_and_buckets_by_month`：`report_at(today="2026-11-19")`，`from="2026-11-19", to="2026-10-25"`（倒序）+ `Month` → `from=="2026-10-25"`、`buckets` 键为 `["2026-10","2026-11"]`、`days == 26`；`from="2020-01-01"` → `from == "2025-10-16"`（今天 − 399 天）、`days == 400`
  - `report_filters_route_and_model_but_lists_all_options`：`route=Some("codex")` → `totals` 只含 codex，`routes` 仍列出全部出现过的线路，`models` 仍列出全部模型
  - `cache_hit_inputs_are_separate`：`Totals` 的 `input / cache_read / cache_write` 分别等于 `Agg.in / cr / cw+cw1h` 之和
  - `export_csv_writes_bom_and_header`：导出到临时目录 → 文件以 `EF BB BF` 开头，首行 `period,route,model,requests,errors,avg_ms,input_uncached,cache_read,cache_write_5m,cache_write_1h,output,cost_usd`，`codex` 行 `cost_usd` 为空、`claude-opus-5-5` 行为 `24.000000`；文件名 `cc-bridge-usage-<from>_<to>.csv`
- [ ] **Step 2: 运行确认失败**：`cd src-tauri && CARGO_TARGET_DIR=../target-usage cargo test --lib stats::`
- [ ] **Step 3: 实现**签名；`days` 只用 jiff 的 `civil::Date` 计算；空桶不输出（前端补零）。
- [ ] **Step 4: 运行通过**：同 Step 2；再跑 `cd src-tauri && CARGO_TARGET_DIR=../target-usage cargo build` 确认命令注册能编译
- [ ] **Step 5: 提交**：`git commit -am "feat(usage): 新增 usage_report 与 export_usage_csv 命令，Rust 端按价格表折算等价 API 价"`

---

### Task 5: 统计页（布局、筛选、卡片、明细表、文案、mock）

**Files:**
- Modify: `src/api.ts`（类型 `UsageTotals`、`UsageBucket`、`UsageModelRow`、`UsageReport`，`Activity` 加 `cacheReadTokens`、`cacheWriteTokens`）
- Modify: `src/main.ts`（`feedMode: 'stats' | 'req' | 'raw'`，默认 `'stats'`；`statsView()`；状态 `usage`、`usageQ`；`loadUsage()`；`act` 新分支 `usage-gran`、`usage-range`、`usage-bucket`、`usage-export`；`change` 监听 `u-route`、`u-model`、`u-from`、`u-to`；`activity` 事件在统计页可见时 `loadUsage()`；`?mock&feed=req` 预览参数）
- Modify: `src/styles.css`（`.ufilters`、`.unote`、`.ucards`（2×2 grid）、`.ushare`（占比条）、`.utable`、`.uchart`，线路色变量 `--r-claude #D97757`、`--r-codex #10A37F`、`--r-api #6E59D9`）
- Modify: `src/mock.ts`（`usage_report`：按传入 from/to/granularity 用固定种子生成 90 天三线路数据，含一个 `gpt-6.1-sol` 未知价格；`export_usage_csv` 返回 `C:\\Users\\me\\Downloads\\cc-bridge-usage-…csv`）
- Modify: `src/locales/zh.ts` 及其余 9 个语言文件
- Test: `npx tsc --noEmit`、`npm run build`、Edge 无头截图

**Interfaces:**
- Consumes: 命令 `usage_report`、`export_usage_csv`（Task 4 的 camelCase 结构）
- Produces:
  - `function statsView(s: Status): string`（图表处只输出 `<div id="usage-chart-slot" class="uchart"></div>`）
  - `async function loadUsage(): Promise<void>`（自增 `usageSeq`，过期响应丢弃；完成后 `shouldRerender()` 为真才 `render()`）
  - `usageQ: { gran: 'today' | 'day' | 'month'; range: 7 | 30 | 90 | 'custom'; from: string; to: string; route: string; model: string; bucket: string | null }`
  - 新文案键：`feedStats`、`usageToday`、`usageDaily`、`usageMonthly`、`usageLast {n}`、`usageCustom`、`usageAllRoutes`、`usageAllModels`、`usageExport`、`usageExported {path}`、`usageUnpriced {models}`、`usageCost`、`usageApiPrice`（「等价 API 价」）、`usageDailyAvg {v}`、`usageRequests`、`usageTokens`、`usageInOut {in} {out}`、`usageHitRate`、`usageTokensChart`（「Tokens（无可计价模型）」）、`usageColModel`、`usageColReq`、`usageColTokens`、`usageColHit`、`usageColCost`、`usageEmpty`、`routeGpt`（「GPT（Codex 订阅）」）

- [ ] **Step 1: 先写类型与 mock，让 tsc 失败**：在 `api.ts` 加类型，`main.ts` 里引用尚未实现的 `statsView` → `npx tsc --noEmit` 报错（确认失败）
- [ ] **Step 2: 实现** `statsView` 与事件：筛选行 → 未知价格提示条 → 2×2 卡片 → 图表占位（「今天」模式不输出占位）→ 选中桶的各模型明细表（未知价格金额显示「—」，命中率分母为 0 显示「—」）。金额格式 `$` + 两位小数（<0.01 且 >0 显示 `<$0.01`）；Tokens 用 `K/M/B` 缩写。
- [ ] **Step 3: 补齐 10 种语言文案**，`npx tsc --noEmit` 通过（`Dict` 缺键会报错）
- [ ] **Step 4: 构建**：`npm run build` 通过
- [ ] **Step 5: 目视检查**（不用 ego-browser）：`npx vite --port 5199` 后台起预览，Edge 无头截图
  `msedge --headless=new --window-size=600,900 --screenshot=C:/…/scratchpad/stats.png "http://localhost:5199/?mock=ok&tab=feed"`，
  检查：筛选行一行内不换行溢出、卡片 2×2、提示条文案完整、明细表不横向滚动；`?lang=en`、`?lang=de` 再各截一张看长文案。截完结束 vite 进程。
- [ ] **Step 6: 提交**：`git add src && git commit -m "feat(usage): 动态页新增统计分段：筛选、花费/请求/Tokens/命中率卡片与模型明细"`

---

### Task 6: ECharts 堆叠柱状图与重绘复用

**Files:**
- Modify: `package.json` / `package-lock.json`（`npm i echarts@^5`）
- Create: `src/usage-chart.ts`
- Modify: `src/main.ts`（`renderMain` 在 innerHTML 之后、恢复 `still` 之后调用 `mountUsageChart`；柱子点击回调设置 `usageQ.bucket` 并 `render()`）
- Test: `npx tsc --noEmit`、`npm run build`、Edge 无头截图两张对比

**Interfaces:**
- Consumes: `UsageReport`、`usageQ.bucket`
- Produces:
  - `export interface ChartInput { keys: string[]; series: { route: 'claude' | 'codex' | 'api'; values: number[] }[]; unit: 'usd' | 'tokens'; selected: string | null }`
  - `export function chartInput(r: UsageReport, gran: 'day' | 'month', selected: string | null): ChartInput`（补齐范围内空天 / 空月为 0；`r.totals.costUsd === 0` 时 `unit = 'tokens'`）
  - `export function mountUsageChart(slot: HTMLElement | null, input: ChartInput, onPick: (key: string) => void): void`（`slot` 为 null 时什么都不做；常驻宿主 `slot.replaceWith(host)`；指纹相同不 `setOption`；首次 `animation: true`，之后 `false`；选中桶之外的柱子 `itemStyle.opacity = 0.35`）
  - 模块内：`let host: HTMLDivElement | null`、`let chart: echarts.ECharts | null`、`let lastKey = ''`、`ResizeObserver`

- [ ] **Step 1: 写会失败的调用**：`main.ts` 引入 `mountUsageChart`、`chartInput`，`npx tsc --noEmit` 报模块不存在
- [ ] **Step 2: 安装依赖**：`npm i echarts@^5`
- [ ] **Step 3: 实现** `usage-chart.ts`：`echarts.use([BarChart, GridComponent, CanvasRenderer])`；`series` 按 `claude → codex → api` 固定顺序 `stack: 'all'`，颜色取 Global Constraints；`tooltip: { show: false }`；y 轴按 `unit` 格式化（`$` / `K` `M`）；`chart.on('click', p => onPick(p.name))`。
- [ ] **Step 4: 构建**：`npx tsc --noEmit` 与 `npm run build` 通过；对比 `dist/assets/*.js` 体积，记录引入前后增量（预期 < 400 KB 未压缩，超出则检查是否误引了 `echarts` 全量入口）
- [ ] **Step 5: 重绘不闪验证**：mock 预览统计页，在浏览器控制台（或临时 `?mock&tick=1` 每秒触发一次 `render()` 的 mock 开关）连续重绘 10 次，Edge 无头隔 3 秒截两张，图表区域像素一致、无空白帧；切到「请求」再切回「统计」，图表立即出现、无重新入场动画。点击柱子后明细表切换到该天。
- [ ] **Step 6: 提交**：`git add package.json package-lock.json src && git commit -m "feat(usage): 每日/每月金额按线路堆叠柱状图，整页重绘时复用常驻 ECharts 实例"`
