# 自定义 Claude API 接入设计（2026-11-18）

## 为什么做

CC Bridge 现在让 Claude Code 在同一个 `/model` 列表里同时用 Claude 订阅（透传）和 ChatGPT 订阅（Codex 翻译）。
有的用户手上还有自己的 Claude API（官方 key、Bedrock / Vertex 转发、第三方中转站），想把它也接进来用，
同时**不影响**原来的订阅透传。

Claude Code 只能吃真正的 Claude：它依赖 Anthropic Messages 协议、带签名的 thinking 块、Claude 的工具调用习惯。
很多非 Claude 模型（Qwen、DeepSeek、Kimi 等）也提供「Anthropic 兼容接口」，格式长得一样，但接进 Claude Code
会用到一半出错或效果变差。所以**保存前必须校验对方真的是 Claude**，不是的直接拒绝，不让进 `/model` 列表。

## 目标

- 用户在客户端里添加若干条「自己的 Claude API」：Base URL、API Key、模型名（如 `claude-opus-5-5`）。
- 每条通过校验后，作为单独一项出现在 Claude Code 的 `/model` 列表，选它就走用户自己的 API。
- 校验能挡住格式兼容但不是 Claude 的接口（Qwen、DeepSeek 等的 Anthropic 兼容端点）。
- 原有 Claude 订阅透传、Codex 路由、接管 / 撤销逻辑行为不变。

## 非目标

- 不分辨同代 Claude 内部的型号替换（标 Opus 实际给 Sonnet）。同代分词器相同、签名都是真的，无法可靠区分（方案 C，已否决）。
- 不做非 Claude 模型的适配 / 翻译。
- 不做用量、计费统计。

## 反目标（技术上能跑但算失败）

- 自定义 API 的请求带上了用户的订阅 OAuth token，或订阅请求带上了用户的 API key（凭证串线）。
- 订阅透传的请求体被改动，破坏 prompt cache 前缀。
- API key 出现在日志、`settings.json`、错误提示、请求统计里。
- 校验结论是「真 Claude」但实际是兼容接口（误放）。宁可误拒，不可误放。

## 方案（已选 B）

| 方案 | 做法 | 取舍 |
|---|---|---|
| A 只查格式 | 发测试请求，检查 `msg_` ID、`content` / `usage` 结构、SSE 事件顺序 | 简单，但 Qwen 等兼容接口都能过，达不到目的 |
| **B 格式 + 两个 Claude 独有指纹** | A 的基础上：① thinking 签名篡改测试 ② 分词计数指纹 | **用户选择**。签名由 Anthropic 加密生成并在服务端校验，兼容接口无法伪造；分词指纹进一步排除用别家分词器的模型 |
| C B + 型号替换检测 | 试图区分同代 Opus / Sonnet | 无可靠技术手段，误判多，否决 |

### 已实测的前提（2026-11-18，经订阅透传到 api.anthropic.com）

1. **签名篡改**：`claude-haiku-4-5` 开启 thinking，拿到 624 字符签名。原样回放 → 200；改动 1 个字符回放 →
   `400 messages.1.content.0: Invalid \`signature\` in \`thinking\` block`。
2. **分词计数**：同一段固定文本（中英文、数字、符号、emoji 混合），`count_tokens` 结果按型号不同：
   haiku-4-5 / sonnet-4-6 = 60，sonnet-5 / opus-4-8 / opus-5 = 66，sonnet-5-5 / opus-5-5 / fable-5-1 = 68，opus-4-7 = 71。
   结论：指纹必须按型号建对照表。
3. **`api-` 前缀不会让 Claude Code 降级**：用假接口抓 Claude Code 2.1.327 的请求，模型名分别为 `claude-opus-5-5`、
   `api-claude-opus-5-5`、`custom-claude-opus-5-5` 时，请求参数完全相同：`thinking: {type: adaptive, display: omitted}`、
   `max_tokens: 128000`、`context_management`、effort，以及同样 8 个 `anthropic-beta` 标志、14 个工具。
   Claude Code 是在名字里识别型号的，所以**选单名必须包含真实的 `claude-…` 型号名**（`api-<model>` 满足）。
4. **未测成**：opus-5-5 / sonnet-5-5 / fable-5-1 的签名篡改，订阅直连被限流（429）。实现阶段必须用真实 API key 验证（见第 4 节）。

## 设计

### 1. 配置

`AppConfig`（`src-tauri/src/config.rs`）新增：

```rust
pub custom_apis: Vec<CustomApi>,

pub struct CustomApi {
    pub id: String,            // 内部 ID（uuid），用于界面增删改
    pub base_url: String,      // 如 https://api.anthropic.com 或中转站地址（不含 /v1/messages）
    pub api_key: String,
    pub model: String,         // 上游真实模型名，如 claude-opus-5-5；只接受 Anthropic 官方格式（见下）
    pub auth_style: AuthStyle, // 校验时探测得出：XApiKey | Bearer
    pub verified_at: Option<u64>, // 校验通过时间（unix 秒）；None = 未通过，不路由、不进选单
}
```

- 存在客户端 app data 目录的配置文件里（和现有配置同一个文件）。**不写进** Claude Code 的 `settings.json`。
- 模型名只接受 Anthropic 官方格式：`claude-` 开头，只含小写字母、数字、`-`（可带 `-YYYYMMDD` 日期后缀）。
  Bedrock / Vertex 风格的 ID（如 `us.anthropic.claude-…-v1:0`、`…@2025…`）保存时直接拒绝并说明原因（我的决定：
  Bedrock / Vertex 原生接口本来就不是 `/v1/messages` 协议；中转站对外基本用官方名；带 `anthropic` 的名字还会让
  `is_claude_model` 误判，导致撤销接管时不恢复 `settings.json`）。
- 返回给前端的状态里，api_key 只给掩码（如 `sk-ant-…a1b2`）；编辑时留空表示不改 key。

### 2. 选单名称与路由

- 每条自定义 API 在 `/model` 列表里的模型名为 `api-<model>`，同一模型名配了多条时用 `api-<model>-2`、`-3` 区分；
  显示名 `API <model>`，描述 `自己的 API · <base_url 的主机名>`。
- **不能直接用 `claude-` 开头的名字**：`registry.rs` 的 `provider_for_model` 会把所有 `claude-*` 和 opus/sonnet 等别名
  送去订阅透传。
- `provider_for_model` 在判断 `claude-` 之前，先查是否命中某条已校验的自定义 API 名，命中就交给新的 `CustomApiProvider`。
- `claude_settings.rs` 的接管 / 撤销沿用现有机制：`TakeoverRecord.models` 记录本客户端写入的所有条目，
  `model_entry` 按名字前缀区分生成 Codex 条目或 API 条目。`is_claude_model` 需确保 `api-` 开头的名字被视为
  「本客户端写入的」，撤销时能恢复（`api-…` 不以 claude 开头，现有逻辑已满足，补测试确认）。

### 3. 转发（CustomApiProvider）

基于现有 `AnthropicProvider` 的透传逻辑，差别：

- 目标地址 = 该条的 `base_url` + 原始 `path_and_query`。
- 请求体只改 `model` 字段：`api-<model>[-n]` → 真实 `model`。其余字节不动。
- **凭证替换**：去掉入站的 `authorization`、`x-api-key`，以及 `anthropic-beta` 里的 `oauth-*` 标志；
  按 `auth_style` 加上用户的 key（`x-api-key: <key>` 或 `authorization: Bearer <key>`）。
- 其余头（`anthropic-version`、其他 `anthropic-beta` 标志）照常转发。
- 走客户端配置的出口代理（同 `AnthropicProvider::new`）。
- thinking 签名的 rehydrate 处理（`sanitize_anthropic_request`）同样适用：会话中途从 Codex 切过来时避免 400。
- 跨来源切换的已知限制：订阅 Claude 产生的 thinking 签名，发到用户 API 那边如果背后是同一个 Anthropic 账号体系
  以外的部署（如 Bedrock），可能报签名无效；Claude Code 会自动去掉 thinking 重试。记录为已知限制，不额外处理。
- `count_tokens` 同样转发到用户 API。

### 4. 校验流程

用户点「保存并校验」或「重新校验」时，后端依次做（任一步失败即停，给出具体原因）：

1. **连通与认证**：`POST {base_url}/v1/messages`，最小请求（`max_tokens: 16`，一句话）。
   先用 `x-api-key`，401/403 再试 `Authorization: Bearer`；记录可用的那种为 `auth_style`。都不行 → 「认证失败」。
2. **格式检查**：响应 `type == "message"`、`id` 以 `msg_` 开头、`role == "assistant"`、`content` 为数组、
   `usage.input_tokens` / `output_tokens` 为整数、`stop_reason` 存在。再用 `stream: true` 发一次，
   检查 SSE 事件序列 `message_start → content_block_* → message_delta → message_stop`。不符 → 「不是 Claude 格式的接口」。
3. **分词指纹**：对照表每个型号存两列基准值：`count_tokens` 结果、固定文本走 `/v1/messages`（`max_tokens: 16`）的
   `usage.input_tokens`。先调 `count_tokens`，非 200、结构不对或数值不等时，再用 `/v1/messages` 的 `usage.input_tokens`
   比对第二列。**两者有一个完全相等即通过**（我的决定：很多中转站的 `count_tokens` 是本地估算的，只比第一列会误拒背后确实是
   真 Claude 的中转站；签名测试仍是硬门槛）。
   - 模型名不在对照表里：跳过此项，界面提示「该型号没有分词基准，仅凭签名判断」，不算失败。
   - 两列都不相等 → 「分词结果和 Claude 不一致，可能不是 Claude 模型」。
4. **签名篡改**：开启 thinking 发一题，取回 thinking 块与签名；把签名改一个字符连同历史回放。
   - 请求形状：按型号决定。旧型号（haiku-4-5 等）用 `{type: enabled, budget_tokens: 1024}`；新型号用 Claude Code 实际发的
     `{type: adaptive, display: omitted}` 加 `output_config.effort: high`，题目选需要推理的（避免 adaptive 判断「不用想」而不出
     thinking 块）。`redacted_thinking` 块同样视为有签名可测。拿不到 thinking 时换一题重试一次再判失败。
   - **实现阶段必做**：用真实 Anthropic API key 对 opus-5-5、sonnet-5-5、fable-5-1、haiku-4-5 各跑一遍，确认篡改返回 400、
     原样回放返回 200，并把实测得到的两列分词基准写进对照表。
   - 回放返回 400 且错误信息含 `signature` → 通过。
   - 回放仍返回正常回答 → 「签名没有被校验，不是真正的 Claude」。
   - 第一步就拿不到 thinking 块或签名为空 → 失败，「接口不支持 Claude 的思考签名」。
5. **模型名核对**：响应里的 `model` 应等于或以用户填的模型名开头（如 `claude-haiku-4-5` 对应
   `claude-haiku-4-5-20251001`）。不符 → 失败，「接口返回的模型是 xxx，和填写的不一致」。

全部通过才写入 `verified_at`，并把它加进 `/model` 列表。整个过程约 4–6 个小请求，用用户自己的 key，几美分以内。
校验期间界面显示逐项进度（✓ / ✗ / 跳过），失败项显示原因。

**指纹对照表**：放在代码里（如 `proxy/providers/custom_api/fingerprint.rs`），键为模型名前缀，值为固定文本的期望 token 数。
初始数据用订阅透传实测生成（附生成脚本或单元测试注释说明如何重测）。新型号上线后补表；未知型号按第 3 步的「跳过」处理。

### 5. 界面

放在「模型」页（`models` 标签）现有 Codex 模型勾选列表下方，新增「自己的 Claude API」一节：

```
自己的 Claude API                                  [+ 添加]
┌──────────────────────────────────────────────────────────┐
│ API claude-opus-5-5   api.example.com   ✓ 已校验  [编辑][删除] │
│ API claude-sonnet-5-5 api.anthropic.com ✗ 未通过  [编辑][删除] │
└──────────────────────────────────────────────────────────┘
```

添加 / 编辑弹窗：Base URL、API Key（密码框，编辑时留空不改）、模型名，按钮「保存并校验」。
校验中显示 5 项进度，失败时保留填写内容，可改完重试。未通过的条目保留在列表里但不进 `/model`。
文案进 10 种语言的 locales。

## 约束

- 不改动订阅透传路径（`AnthropicProvider`）的行为，订阅相关测试全部保持通过。
- API key 不进日志、不进请求统计、不进前端明文、不进 `settings.json`。
- 开发模式（`tauri dev`，端口 18766，沙盒 settings）下可完整测试。

## 留给实现者

- 保存 / 校验后路由表的生效方式（热更新还是重启代理；现在 `Registry` 只在启动时建一次）。
- 编辑已校验条目时旧配置是否先保留可用、删除后 `-n` 序号是否重排。
- 自定义 API 条目是否受现有 `selected_models` 勾选控制（建议不受，校验通过即进选单）。
- 认证探测遇到 401/403 以外的状态码时的提示文案。

- `CustomApiProvider` 与 `AnthropicProvider` 的代码复用方式（抽公共转发函数还是组合）。
- 掩码格式、uuid 生成方式、弹窗的具体样式（沿用现有 sheet 弹窗）。
- 校验各步的超时（建议单步 30 秒）。
- 校验用固定文本的具体内容（沿用实测用的那段即可）。
