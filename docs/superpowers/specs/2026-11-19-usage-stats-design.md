# 用量统计设计（2026-11-19）

## 意图

「动态」页加一个像「用量看板」那样的统计：经过 CC Bridge 的请求，按天 / 月看花了多少（等价 API 价）、
请求数、Tokens、缓存命中率，按线路、模型筛选和导出。只统计经过 CC Bridge 的请求。

## 目标 / 非目标

目标：
- 代理每完成一次 `/v1/messages` 请求，记下 日期（本地时区）/ 线路 / 模型 / 成功或出错 / 耗时 / 四项 token。
- 按「天 × 线路 × 模型」汇总落盘，保留 400 天；原「近 7 天 N 次」「今天」功能不变。
- Rust 端按内置价格表折算美元（标「等价 API 价」），新 Tauri 命令 `usage_report` 给前端，`export_usage_csv` 导出。
- 动态页分段改为 [统计][请求][原始日志]，统计为默认。

非目标：
- 不统计不走 CC Bridge 的用量；不存逐条请求历史（逐条只有内存里最近 200 条，同现在）。
- 不做自定义价格编辑（下一版）；不按小时统计；不算 web search 等服务端工具费用。

## 现状（调研结论）

- `src-tauri/src/stats.rs`：每 2 秒（`lib.rs` 线程）从 `MonitorHandle::snapshot().recent` 收 `EndpointKind::Messages` 的已完成请求，
  按 `request_id` 去重；`stats.json` 结构 `{"days": {日期: {模型: {count, errors, ms}}}}`，保留 8 天，先写 `stats.json.tmp` 再 rename。
  线路 `route` 由 `provider` 推出：`anthropic`→`claude`，`custom-api`→`api`，其余→`codex`。模型取「请求的 → 实际的」箭头前那段。
- 监控 `CompletedRequest` 只有 `input_tokens` / `output_tokens`，且 `update_token_count` **只增不减（取最大值）**。
  Codex 流式的 `message_start` 先发**估算**的 input（`count_translated_tokens`，含缓存部分），`message_delta` 才是真实值，
  所以监控里的 input 经常是估算值，不能直接拿来算钱。
- Claude 订阅透传（`providers/anthropic/mod.rs::relay`）和自己的 API（`custom_api/provider.rs`）**完全不解析 usage**，
  直接把上游 `bytes_stream()` 交回去，现在这两条线路 token 都是空。
- 所有 `/v1/messages` 响应最后都经过 `server.rs::monitor_response_body`，下游一律是 **Anthropic 协议**
  （Codex 已在 `translate/reducer.rs::map_codex_usage_to_anthropic` 换算好）；body 读完才调 `guard.completed`，
  所以在这里统一解析 usage、在 `completed/failed` 之前上报，统计线程取到的就是终值。
- 前端 `main.ts` 每次 `render()` 用 `app.innerHTML` 整页重绘；`ANIMATED` / `animKey` 让重绘前已在屏上的动画元素加 `still` 不再重播。
  `activity` 事件只在有新请求时推送（最快 2 秒一次）；`status-changed` 也会触发重绘。
- 没有 dialog / fs 插件，`Cargo.toml` 有 `dirs = "6"`，`tauri.conf.json` 开了 `withGlobalTauri`。

## 数据口径

统一成四项：**未命中输入 / 缓存读 / 缓存写 / 输出**。缓存写再分 5 分钟与 1 小时两档（只为算钱，界面合并显示）。

| 来源 | 未命中输入 | 缓存读 | 缓存写 5m | 缓存写 1h | 输出 |
|---|---|---|---|---|---|
| Anthropic usage（透传、自己的 API） | `input_tokens`（本身不含缓存） | `cache_read_input_tokens` | `cache_creation.ephemeral_5m_input_tokens`；无细分时 = `cache_creation_input_tokens` | `cache_creation.ephemeral_1h_input_tokens`；无细分时 0 | `output_tokens` |
| OpenAI / Codex usage | `input_tokens − input_tokens_details.cached_tokens`（OpenAI 的 input 含缓存） | `cached_tokens` | 0（协议没有） | 0 | `output_tokens` |

- Codex 线路的换算已在 `map_codex_usage_to_anthropic` 完成（`input_tokens = total − cached`，`cache_read_input_tokens = cached`，
  `cache_creation_input_tokens = 0`），统计在下游按 Anthropic 口径读即可，不再重复换算。
- SSE：`message_start.message.usage` 与 `message_delta.usage` 都读，**每个字段后到的覆盖先到的**（不取最大值）。
  这样 Codex 的估算 input 会被 `message_delta` 的真实值替换；Anthropic 旧版 `message_delta` 只带 `output_tokens` 时，input / 缓存沿用 `message_start` 的。
- 非流式 JSON：读顶层 `usage`。
- 出错的请求：计次数和出错数；解析到多少 token 就记多少（中途断流可能有部分 token）。
- 总 Tokens = 未命中输入 + 缓存读 + 缓存写 + 输出；「输入」= 未命中输入 + 缓存读 + 缓存写。
- 缓存命中率 = 缓存读 ÷（未命中输入 + 缓存读 + 缓存写），分母为 0 显示「—」。

## 价格表（美元 / 百万 token，内置常量）

| 模型键 | 输入 | 输出 | 缓存读 | 缓存写 5m | 缓存写 1h |
|---|---|---|---|---|---|
| claude-opus-5-5 | 4 | 20 | 0.2 | 5 | 8 |
| claude-sonnet-5-5 | 2 | 10 | 0.1 | 2.5 | 4 |
| claude-sonnet-5 | 2 | 10 | 0.2 | 2.5 | 4 |
| claude-fable-5-2 | 8 | 40 | 0.2 | 10 | 16 |
| claude-fable-5-1 | 10 | 50 | 0.25 | 12.5 | 20 |
| claude-opus-5 | 5 | 25 | 0.5 | 6.25 | 10 |
| claude-haiku-5-5 | 0.1 | 0.5 | 0.01 | 0.125 | 0.2 |
| claude-haiku-4-5 | 1 | 5 | 0.1 | 1.25 | 2 |
| claude-opus-4-8 | 5 | 25 | 0.5 | 6.25 | 10 |
| claude-opus-4-7 | 5 | 25 | 0.5 | 6.25 | 10 |
| claude-sonnet-4-6 | 3 | 15 | 0.3 | 3.75 | 6 |

- Haiku 5.5 只取提示 ≤100K 的档；>100K 档（输入 0.5 / 输出 2.5）忽略——汇总后已不知道单次提示长度，长提示会少算。
- 匹配：小写 → 去掉 `[...]` 后缀（如 `[1m]`）→ 去掉 `-YYYYMMDD` / `@YYYYMMDD` 日期后缀 → 与模型键**完全相等**。
  不做「最长前缀」：`claude-sonnet-5-6` 这种新型号宁可算未知，也不能套成 Sonnet 5 的价。
- **只有 `claude`（Claude 订阅透传）线路计价**。`codex`（GPT）一律未知价格；`api`（自己的 API）也一律未知价格：
  用户自己的 API 可能是中转站或其它厂商的 Claude 兼容接口，实际单价与官方不同，套官方价会误导。
- 未知价格模型：token 照常统计，金额按 0 计、不进总花费；报表返回 `unpricedModels`，前端顶部提示
  「含未知价格模型：xxx。这些模型的 token 已统计，但不计入金额。」

## 存储与迁移

新文件 `<data_dir>/usage.json`，原子写入沿用 `stats.json` 的「写 `.tmp` 再 rename」：

```json
{ "version": 2,
  "days": { "2026-11-19": { "claude": { "claude-opus-5-5": {"n":12,"e":1,"ms":20400,"in":800,"cr":120000,"cw":3000,"cw1h":0,"out":5600} } } } }
```

- 键：日期（本地时区 YYYY-MM-DD）→ 线路（`claude`/`codex`/`api`）→ 模型（同 `Activity.model`，即 Claude Code 里选的名字；自己的 API 是 `api-<model>`）。所有数字字段 `#[serde(default)]`。
- 保留 400 天：每次有新请求时删掉 `日期 <= 今天 − 400 天` 的。400 天 × 3 线路 × 约 10 模型，文件 < 1 MB。
- 迁移：启动时 `usage.json` 不存在而 `stats.json` 存在 → 一次性导入旧 8 天的 count/errors/ms（token 为 0），
  线路按模型名推断：`api-` 开头→`api`，`claude-` 开头→`claude`，其余→`codex`，随后写出 `usage.json`。
  **不再写、也不删除 `stats.json`**：回退到旧版本时旧代码仍能读它（旧代码读不懂新结构会清空重写，所以不复用同一个文件）。
- 「近 7 天 N 次」「今天」改为从 `usage.json` 汇总（跨线路求和），`Overview` 结构不变。

## 接口

监控层（`proxy`）：
- 新 `proxy/usage_tap.rs`：`TokenUsage`（五个 u64）与增量解析器 `UsageTap`（SSE 按行、跨 frame 拼接；JSON 累积到 2 MiB 上限后在结尾解析）。
- `MonitorEvent::UsageFinal { request_id, usage }` + `MonitorHandle::usage_final()`；`ActiveRequest` / `CompletedRequest` 加 `pub usage: Option<TokenUsage>`。
- `monitor_response_body` 每个 data frame 喂给 tap，结束时先 `usage_final` 再 `completed` / `failed`。

客户端（Tauri 命令，注册在 `lib.rs`）：

```rust
#[tauri::command] fn usage_report(st, from: String, to: String, granularity: String /* "day"|"month" */,
                                  route: Option<String>, model: Option<String>) -> Result<UsageReport, String>;
#[tauri::command] fn export_usage_csv(st, from, to, granularity, route, model) -> Result<String /* 文件路径 */, String>;
```

`UsageReport`（camelCase）：`from, to, granularity, days`（范围天数，算日均用）、`totals: Totals`、`byRoute: {route: Totals}`、
`buckets: [{ key: "2026-11-19"|"2026-11", byRoute: {route: Totals}, models: [{route, model, totals, priced}] }]`、
`unpricedModels: [String]`、`routes / models`（范围内出现过的，供下拉；不受 route/model 筛选影响）。
`Totals`：`count, errors, ms, input, cacheRead, cacheWrite, output, costUsd`（`costUsd` 只含已知价格部分）。

- `from > to` 交换；早于「今天 − 399 天」的截到这天；晚于今天的截到今天。
- 日均 = 合计 ÷ `days`（含没有请求的天，截至今天）。
- CSV 写到下载目录（`dirs::download_dir()`，取不到用 data_dir），文件名 `cc-bridge-usage-<from>_<to>.csv`，UTF-8 带 BOM；
  列 `period,route,model,requests,errors,avg_ms,input_uncached,cache_read,cache_write_5m,cache_write_1h,output,cost_usd`，
  未知价格 `cost_usd` 留空。不引入 dialog 插件，前端 toast 显示路径。

## 前端布局与交互（窗口约 600px，竖排）

1. 分段：[统计][请求][原始日志]，`feedMode` 加 `'stats'` 并默认。
2. 筛选行：分段 [今天][每日][每月]；时间范围下拉（近 7 / 30 / 90 天 / 自定义）+ 自定义时两个 `<input type="date">`；
   「全部线路▾」「全部模型▾」（复用 `renderSelect`）；「导出 CSV」按钮。「今天」模式隐藏时间范围。
3. 未知价格提示条（有 `unpricedModels` 时）。
4. 2×2 卡片：总花费（等价 API 价，`$12.34`）+ 日均 + 按线路占比条；请求数 + 日均 + 占比条；
   总 Tokens（输入 · 输出）+ 占比条；缓存命中率 + 各线路命中率。
5. 柱状图：每日 / 每月金额按线路堆叠。**范围内 `costUsd` 全为 0 时改按总 Tokens 堆叠**，图标题写「Tokens（无可计价模型）」。
   「今天」模式不画图（没有小时数据），直接显示今天各模型明细表。
6. 点击柱子：选中该桶，其余柱子变淡，下方显示该天 / 该月各模型明细表（线路、模型、请求、Tokens、命中率、金额，未知价格显示「—」）；
   再点同一柱子取消。默认选中最后一个有数据的桶。

线路固定配色（全图、占比条、明细表色点一致，深浅色主题相同）：
Claude 订阅 `#D97757`、GPT（Codex 订阅）`#10A37F`、自己的 API `#6E59D9`。

## 图表与重绘方案

- ECharts 按需引入：`echarts/core` + `BarChart` + `GridComponent` + `CanvasRenderer`（不引 Tooltip / Legend 组件，tooltip 天然关闭；`tooltip: { show: false }` 也写上）。
- 新模块 `src/usage-chart.ts` 持有**一个常驻的图表宿主 div 和 ECharts 实例**，不随 innerHTML 销毁：
  模板里只输出占位 `<div id="usage-chart-slot">`；`renderMain` 在 innerHTML 之后调用 `mountUsageChart()`，
  把常驻宿主 `slot.replaceWith(host)` 搬进新 DOM。canvas 跟着节点走，位图不丢，不闪。
- 数据指纹（`JSON.stringify` 图表输入）没变就不 `setOption`；变了用 `notMerge: true`，只有首次渲染开动画。
- 离开统计页时宿主只是脱离 DOM，不 `dispose`；`ResizeObserver` 在宿主尺寸变化时 `chart.resize()`。
- 数据：进入统计页、改筛选、`activity` 事件（统计页可见时）各取一次 `usage_report`；用自增序号丢弃过期响应。
  日期输入框有焦点时 `shouldRerender()` 已经会跳过重绘，不冲掉输入。

## 风险

- 解析每个 SSE frame 有开销：只对以 `data:` 开头且含 `"usage"` 的行做 JSON 解析；未完成的行缓冲上限 1 MiB，超出丢弃到下一个换行。
- 监控 `recent` 有容量上限，2 秒内完成的请求超过容量会漏计——现状就有，本次不改。
- Haiku 5.5 长提示档、web search 费用不算；价格表写死，官方调价要发版（下一版做价格编辑）。
- 旧 8 天数据的线路是按模型名推断的，且没有 token。
- 跨午夜的请求按**完成时间**的本地日期记。
- ECharts 增加前端体积（按需引入后约 300 KB 级，未压缩）。
