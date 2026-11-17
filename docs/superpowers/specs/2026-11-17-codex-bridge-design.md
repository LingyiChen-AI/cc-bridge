# Codex Bridge 设计文档（2026-11-17）

一个跨平台托盘客户端，唯一职责：在本机常驻运行 Codex→Claude Code 模型代理，让普通 `claude` 命令能在 `/model` 里选用 Codex（ChatGPT 订阅）模型。

## 1. 背景与为什么做

2026-11-17 在本机用命令行方式接入了开源代理 [fcakyon/claude-code-with-codex](https://github.com/fcakyon/claude-code-with-codex)（二进制 `claude-codex` v0.8.0，MIT），一天内踩了三个坑，最后整套配置被删掉：

| 现象 | 根因 |
|---|---|
| 代理时有时无，关掉某个终端 / Orca 窗口后代理消失 | 代理是从终端里拉起的子进程，被挂在该窗口的 Windows Job 对象里，窗口关闭时一起被杀 |
| Claude Code 选 Codex 模型报 `401 No Codex credentials. Log in with the Codex CLI (codex login)` | 代理用环境变量 `HOME` 定位 `~/.codex/auth.json`（见上游 `src/providers/codex/auth/token_store.rs::codex_auth_file`）。Windows 默认没有 `HOME`，从 Codex 桌面版 / PowerShell 拉起的代理找不到文件，即使 `codex` CLI 本身登录正常 |
| 所有 Claude Code 窗口报 `ECONNREFUSED` | `~/.claude/settings.json` 里写死了 `env.ANTHROPIC_BASE_URL=http://127.0.0.1:18765`，代理一旦不在，所有 Claude Code（包括 Claude 自家模型）全部不可用 |

另外：直连 OpenAI 在本机会超时，代理进程必须走出口代理（本机为 `http://127.0.0.1:7890`）；Codex 桌面版会注入一堆 `CODEX*` 变量和它自己的 `HTTP(S)_PROXY=127.0.0.1:52721`，被代理继承后行为不可预期。

用户诉求：做一个有界面的客户端，点叉只缩到托盘、后台常驻，代理就一直在；把上面的坑都在客户端里兜住。

## 2. 目标 / 非目标 / 反目标

**目标**
- Windows / macOS / Linux 三平台托盘常驻，开机自启，单实例。
- 代理进程由客户端独立拉起、监护，崩溃自动重启，不受任何终端窗口生命周期影响。
- 启动代理时环境变量确定、干净：始终有 `HOME`，出口代理可配置，不继承 `CODEX*` / 外部代理变量。
- 由客户端管理 Claude Code 的接管开关（`settings.json` 中的 `ANTHROPIC_BASE_URL` 和 Codex 模型选单），代理不在时不让 Claude Code 卡死在 ECONNREFUSED 上。
- 界面上一眼看清三盏灯：代理是否在跑、Codex 登录是否可用、Claude Code 是否已接管；出问题给出人话原因和下一步操作。

**非目标（v1 不做）**
- 不重写协议转换层（Anthropic Messages ↔ OpenAI Responses），完全复用上游二进制。
- 不支持上游的 Kimi / Cursor / Grok 后端，只做 Codex。
- 不自己实现 ChatGPT 登录流程，只检测状态并引导用户跑 `codex login`。
- 不做客户端自动更新、不做代码签名 / 公证（mac 首次打开需右键打开，文档里说明）。
- 不做多账号切换。

**反目标（即使能跑也算失败）**
- 破坏 Claude 订阅登录：任何情况下不写 `ANTHROPIC_AUTH_TOKEN` / `ANTHROPIC_API_KEY`（上游明确说会覆盖 Claude 订阅登录导致 Claude 路由 401）。
- 把 `~/.claude/settings.json` 写坏或丢掉用户其它配置（hooks、插件、状态栏、代理等）。
- 客户端自己读写 / 修改 `~/.codex/auth.json`（令牌刷新与回写交给上游代理，它已实现单飞锁 + 原子写）。
- 客户端退出后在后台留下孤儿代理进程。

## 3. 约束

- 上游 `claude-codex` 每个版本发布 6 个平台二进制：`darwin-amd64/arm64`、`linux-amd64/arm64`（tar.gz）、`windows-amd64/arm64`（zip），每个带 `.sha256`。
- 上游代理监听 `127.0.0.1:<port>`（默认 18765，`serve --port` 可改），`/v1/models` 返回可用模型；`CCP_LOG_STDERR=1` 时 stderr 输出 JSON 行日志；错误详情写在 `~/.local/state/claude-code-proxy/errors/`。
- 上游令牌刷新逻辑：按 `access_token` 的 `exp` 判断，提前一定余量用 `refresh_token` 调 `https://auth.openai.com/oauth/token` 刷新并回写 auth.json；文件不存在 / 无 `tokens.access_token` 时报 `No Codex credentials`。
- 上游 auth.json 路径优先级：`CCP_CODEX_AUTH_FILE` > `CODEX_HOME/auth.json` > `HOME/.codex/auth.json` > 相对路径 `.codex/auth.json`。
- Claude Code 读取 `~/.claude/settings.json` 的 `env` 和 `modelPicker`，**只在启动时读取**；已打开的窗口不会感知改动，必须重开。

## 4. 方案比较（已选 A）

| 方案 | 做法 | 优点 | 缺点 |
|---|---|---|---|
| **A（选定）** | Tauri 2 托盘客户端 + 上游二进制作为子进程（sidecar） | 安装包约 10MB；托盘 / 自启 / 单实例有官方插件；子进程挂了直接重启；跟进上游只需换二进制 | 多管一个子进程 |
| B | Tauri，把上游 `lib.rs` 当 Rust 库编进同一进程 | 单进程 | 上游库接口不承诺稳定，每次跟版本可能要改代码；崩溃会拖垮整个客户端 |
| C | Electron + 子进程 | 前端开发熟 | 安装包 150MB+，常驻内存数百 MB，与「一个小代理」不相称 |

用户选 A，理由：目标是「代理一直在、坏了能自己恢复」，子进程模型最容易做监护与重启，也最容易跟进上游版本。

## 5. 设计

### 5.1 组成

```
codex-bridge/
  src-tauri/            Rust：托盘、窗口、子进程监护、settings.json 管理、状态检测
  src/                  前端：单个主窗口（状态 + 设置 + 日志）
  binaries/             构建时按平台下载的 claude-codex-<target-triple>[.exe]（不入库）
  scripts/fetch-sidecar 下载指定上游版本并校验 sha256 的脚本
```

上游版本在一个配置文件里钉死（初始 `v0.8.0`），构建时下载、校验 sha256、按 Tauri `externalBin` 命名规则放进 `binaries/`。

### 5.2 代理监护（核心）

- 代理与「接管」开关相互独立（已定）：客户端运行期间代理始终在跑（登录检测、模型列表都依赖它）；「接管」只决定是否改 settings.json。界面不提供「停止代理」，要停就退出客户端。
- 客户端启动即拉起代理，参数 `serve --no-monitor --port <port>`，并设 `CCP_LOG_STDERR=1`，捕获 stdout/stderr。
- **环境变量白名单式构造**，不直接继承客户端自身环境：
  - 从当前环境拷贝，然后删除所有 `CODEX*`、`CCP_*`、`ANTHROPIC_*`、`HTTP_PROXY/HTTPS_PROXY/ALL_PROXY/NO_PROXY`（大小写各形态）。
  - 设置 `HOME` = 用户主目录（Windows 取 `USERPROFILE`）。
  - 用户在设置里填了「Codex 目录」才设 `CODEX_HOME`。
  - 用户在设置里填了「出口代理」才设 `HTTP_PROXY/HTTPS_PROXY`，并固定 `NO_PROXY=localhost,127.0.0.1,::1`。
- **生命周期绑定客户端**：Windows 下把子进程放进客户端自建的 Job 对象并设置 `KILL_ON_JOB_CLOSE`，客户端无论怎么退出子进程都随之结束；macOS / Linux 用进程组 + 退出时 kill，Linux 额外设 `PR_SET_PDEATHSIG`。
- 启动前检查端口：若已被占用，判断占用者是不是 `claude-codex`——是则提示「已有一个代理在跑（可能是旧的手动配置）」并提供「结束它并由我接管」按钮；不是则提示端口冲突，让用户改端口。
- **健康检查**：每 10 秒请求 `GET /v1/models`（不经过任何代理）。连续 3 次失败或进程退出 → 自动重启，退避 1s / 2s / 5s / 10s / 30s 封顶；1 分钟内重启超过 5 次停止自动重启，红灯并显示最后几行 stderr。
- 日志：内存里保留最近 500 行，界面可看、可一键复制；不落盘（上游自己的 `~/.local/state/claude-code-proxy` 已有完整日志）。

### 5.3 Codex 登录状态

- 按上游同样的优先级定位 auth.json（`CODEX_HOME` 设置项 > `~/.codex/auth.json`）。
- 只读检测，不修改：文件是否存在、`tokens.access_token` 是否存在、其 JWT `exp` 还有多久、是否有 `refresh_token`。
- 额外从代理日志里识别 `request_failed` 且 `status=401` 的条目，若出现则把登录灯标红，原因原样展示上游 message。
- 状态与操作：
  - 绿：已登录，显示「令牌到期 MM-DD HH:mm，代理会自动续期」。
  - 黄：令牌已过期但有 refresh_token —— 正常，代理下次请求会续期；无需操作。
  - 红：文件不存在 / 无 access_token / 代理返回 401 → 按钮「打开终端运行 codex login」（各平台调用系统终端执行 `codex login`）；未安装 `codex` CLI 时提示安装方法链接。
- 文件变更监听：auth.json 变化后立即重新检测，并在登录从红变绿时自动重启一次代理。

### 5.4 Claude Code 接管开关

由一个「接管 Claude Code」开关控制 `~/.claude/settings.json`（Claude 目录可在设置中改）：

- **打开时**：等代理健康后才写入；写入内容只有两处：
  - `env.ANTHROPIC_BASE_URL = "http://127.0.0.1:<port>"`
  - `modelPicker.options`：从代理 `/v1/models` 取 Codex 模型，生成 `{model, label: "Codex <id>", description: "ChatGPT subscription"}`。2026-11-17 实测：写入 `options` 后 Claude Code 自带的 Claude 模型仍在 `/model` 里，Codex 条目追加在后面（More models 翻页），所以不往里写 Claude 模型。
  - 合并规则（实现者决定，已定）：按条目管理。客户端只增删「`model` 在本客户端已登记列表里」的条目，用户自己加的条目原样保留、顺序不动。上游模型列表变化时（每次代理启动 / 点刷新）重算客户端条目并替换。
- **关闭或退出客户端时**：只删除上面两处中「由本客户端写入的值」——写入时把写入的值记在客户端自己的状态文件里，删除前比对：`ANTHROPIC_BASE_URL` 不等于记录值就不动并提示；`options` 按条目比对，只删仍与记录一致的条目。
- **顶层 `model` 字段（已定）**：用户在 `/model` 选 Codex 模型后，Claude Code 会把它存成默认模型（2026-11-17 实测出现「saved as your default for new sessions」）。撤销接管（包括代理故障时的临时撤销）时，若顶层 `model` 是客户端登记过的 Codex 模型 id，则改回接管前记录的值（没有记录就删除该字段，让 Claude Code 用默认）；恢复接管时不自动改回 Codex 模型。`env.ANTHROPIC_MODEL` 等变量若为 Codex 模型 id 同样处理。
- **退出 = 撤销接管（用户已定）**：托盘「退出」、系统关机 / 注销都执行撤销，把 settings.json 恢复到未接管状态；下次客户端（含开机自启）启动、代理健康后，若「接管」开关是开的再自动写回。开机后几秒内新开的 Claude Code 窗口只有 Claude 模型，可接受。关机信号（Windows `WM_ENDSESSION`、mac / Linux 的终止信号）里只做撤销这一件快事，写文件要同步完成。
- **写入方式**：读 → 改 → 写临时文件 → 原子替换；首次写入前在客户端数据目录保留一份原始备份；JSON 解析失败则拒绝写入并提示，不覆盖。
- **代理挂了怎么办**：自动重启失败进入红灯状态时，自动临时移除 `ANTHROPIC_BASE_URL`，让新开的 Claude Code 窗口至少能用 Claude 模型；代理恢复后再写回。托盘弹通知说明。
- **崩溃恢复**：客户端每次启动先核对 settings.json——若里面有本客户端写入的 `ANTHROPIC_BASE_URL` 但代理没起来，先移除再启动代理。
- 界面始终提示：「改动只对新开的 Claude Code 窗口生效，已打开的窗口需重开」。

### 5.5 界面

托盘菜单：
```
● 代理运行中 :18765          （状态行，不可点）
● Codex 已登录
● Claude Code 已接管
────────────
打开主窗口
接管 Claude Code      ✓
重启代理
开机自启              ✓
────────────
退出
```

主窗口（单页，约 480×560）：
```
┌ Codex Bridge ─────────────────────────┐
│ 代理        ● 运行中  127.0.0.1:18765  [重启] │
│ Codex 登录  ● 已登录  令牌到期 11-27 15:27     │
│ Claude Code ● 已接管  （新开窗口生效）  [关闭] │
├───────────────────────────────────────┤
│ 可用 Codex 模型（30）  gpt-6.1-sol …   [刷新] │
├───────────────────────────────────────┤
│ 设置：端口 [18765]  出口代理 [http://127.0.0.1:7890] │
│      Codex 目录 [默认]  Claude 目录 [默认]          │
│      开机自启 [✓]  上游版本 v0.8.0                   │
├───────────────────────────────────────┤
│ 日志（最近 500 行）                     [复制] │
└───────────────────────────────────────┘
```
- 点窗口的关闭按钮 = 隐藏到托盘；只有托盘「退出」才真正退出（退出前执行 5.4 的清理）。
- 首次运行引导：检测到 `codex` CLI、auth.json、settings.json 位置，询问出口代理（预填系统代理或空），然后启动代理并询问是否开启接管。

### 5.6 开机自启与单实例

- 用 Tauri 官方 autostart 插件（Windows 注册表 Run 键 / mac LaunchAgent / Linux XDG autostart），默认开启；自启时直接进托盘不弹窗。
- 单实例插件：再次打开时激活已有窗口。

## 6. 交付范围（用户已定）

- v1 以 **Windows x64** 完整实现并验收（本机实测）。
- **macOS / Linux x64**：保证能编译、能打出安装包（dmg / AppImage 或 deb），平台相关代码按设计写好，但暂不实测验收，等有机器再测。Linux 说明里注明 GNOME 需装 AppIndicator 扩展才显示托盘。
- **arm64** 各平台 v1 不出包。

## 7. 留给实现者的决定

- 前端框架（原生 TS / Vue / Svelte 均可，界面只有一页），样式自定，跟随系统深浅色。
- 托盘图标的三态配色与图标设计。
- 各平台「打开终端执行 codex login」的具体实现方式。
- 健康检查间隔、退避参数可在上述量级内微调。
- 构建与打包流水线（本地构建 / CI 矩阵）、安装包格式（msi/nsis、dmg、AppImage/deb）。
- 客户端自身数据目录位置（用 Tauri 默认的 app data 目录）。
