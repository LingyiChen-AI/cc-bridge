# Codex Bridge Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 一个 Tauri 2 托盘客户端，常驻监护上游 `claude-codex` 代理，并安全地接管 / 撤销 Claude Code 的 `settings.json`。

**Architecture:** Rust 后端分成几个纯逻辑模块（环境变量构造、settings.json 接管 / 撤销、Codex 登录检测），每个都有单元测试；另有一个 supervisor 负责拉起 sidecar、健康检查和重启。前端只有一页，用原生 TypeScript + Vite，通过 Tauri command 和事件与后端通信。测试方式（用户要求）：开发过程中不走逐步红绿 TDD，各任务写完代码、能编译即可往下推进；纯逻辑模块的单元测试照常写（列在各任务里），但统一在全部开发完成后集中跑测试、做 Task 6 整体验收并修复。

**Tech Stack:** Rust 1.99、Tauri 2、tauri-plugin-autostart 2、tauri-plugin-single-instance 2、tauri-plugin-notification 2、serde_json（`preserve_order`）、reqwest（blocking 关、rustls）、tokio、windows-sys（Job 对象）、libc（Linux pdeathsig）；前端 Vite + TypeScript（无框架）。

**Spec:** `docs/superpowers/specs/2026-11-17-codex-bridge-design.md`

## Global Constraints

- 上游版本钉死 `v0.8.0`，来源 `https://github.com/fcakyon/claude-code-with-codex/releases/download/v0.8.0/claude-codex-<os>-<arch>.{zip|tar.gz}`，必须用同名 `.sha256` 校验。
- 默认端口 `18765`；代理参数 `serve --no-monitor --port <port>`，环境加 `CCP_LOG_STDERR=1`。
- 子进程环境：删除 `CODEX*`、`CCP_*`（除 `CCP_LOG_STDERR`）、`ANTHROPIC_*`、`HTTP_PROXY/HTTPS_PROXY/ALL_PROXY/NO_PROXY`（不分大小写）；设置 `HOME`；配置了出口代理才设 `HTTP_PROXY`/`HTTPS_PROXY` 和 `NO_PROXY=localhost,127.0.0.1,::1`；配置了 Codex 目录才设 `CODEX_HOME`。
- 永远不写 `ANTHROPIC_AUTH_TOKEN` / `ANTHROPIC_API_KEY`；永远不写 `~/.codex/auth.json`。
- settings.json 只动 `env.ANTHROPIC_BASE_URL`、`modelPicker.options` 里本客户端登记的条目、以及撤销时顶层 `model` / `env.ANTHROPIC_MODEL`（值是登记过的 Codex id 时才改回）。
- 模型条目格式 `{"model": id, "label": "Codex <id>", "description": "ChatGPT subscription"}`。
- 健康检查 `GET http://127.0.0.1:<port>/v1/models`，每 10 秒一次，不走任何代理；连续 3 次失败或进程退出时重启，退避 1/2/5/10/30 秒；60 秒内重启超过 5 次就停止自动重启，并临时撤销接管。
- 点窗口关闭按钮 = 隐藏到托盘；托盘「退出」、关机或注销 = 撤销接管 + 结束代理。
- 界面文案统一中文。提交信息不加 `Co-Authored-By`。
- v1 只在 Windows x64 验收；macOS / Linux x64 能编译、能打包即可。

## Review Focus

1. **settings.json 解析失败**（带 BOM、有注释、尾逗号、空文件）：BOM 要剥掉后正常解析；其余情况拒绝写入并报错，原文件一个字节都不能改。测试见 Task 2。
2. **settings.json 或 `~/.claude` 目录不存在**：接管时创建目录和最小文件；撤销后留下 `{}`，不报错。测试见 Task 2。
3. **端口 18765 已被今天手动起的旧 `claude-codex` 占用**：识别出来，提供「结束它并接管」。在 Task 3 里实现，Task 6 手工验收。
4. **出口代理 7890 没开**：本地 `/v1/models` 仍然健康，但上游请求超时。日志解析要把这种情况归为「出口网络不可用」并在界面提示，不能误报成登录问题。测试见 Task 3。
5. **用户目录含中文或空格**：`HOME` 原样传进去，不能被截断或转码。测试见 Task 2。

---

### Task 1: 脚手架 + sidecar 下载

**Files:**
- Create: 用 `npm create tauri-app@latest` 生成的整套工程（项目名 `codex-bridge`，模板 `vanilla-ts`），放到 `C:\codes\freecall\codex-bridge`，保留已有的 `docs/`
- Create: `scripts/fetch-sidecar.mjs`、`sidecar.json`（`{"version":"v0.8.0"}`）、`.gitignore`（加 `src-tauri/binaries/`）
- Modify: `src-tauri/tauri.conf.json`：`identifier: "dev.freecall.codexbridge"`，`bundle.externalBin: ["binaries/claude-codex"]`，主窗口 `480x600`、`visible: false`、`resizable: false`
- Modify: `package.json`：`"predev"` 和 `"prebuild"` 都先执行 `node scripts/fetch-sidecar.mjs`

- [ ] **Step 1:** 在项目目录 `git init -b master`，提交 spec 和 plan，然后 `git checkout -b feature/codex-bridge-v1`。
- [ ] **Step 2:** 实现 `fetch-sidecar.mjs`：用 `rustc -vV` 取 host triple，映射到上游资产名（`x86_64-pc-windows-msvc`→`windows-amd64.zip`，`aarch64-apple-darwin`→`darwin-arm64.tar.gz`，`x86_64-apple-darwin`→`darwin-amd64.tar.gz`，`x86_64-unknown-linux-gnu`→`linux-amd64.tar.gz`）。下载文件和 `.sha256`，校验通过后解压，得到 `src-tauri/binaries/claude-codex-<triple>[.exe]`。目标文件已存在且哈希一致时跳过。下载走 `HTTPS_PROXY` 环境变量（本机构建时设 7890）。
- [ ] **Step 3:** 运行 `node scripts/fetch-sidecar.mjs`。预期：`src-tauri/binaries/claude-codex-x86_64-pc-windows-msvc.exe` 存在，执行 `--version` 输出 `0.8.0`。
- [ ] **Step 4:** 运行 `npm run tauri dev`。预期：编译通过，进程起来（窗口默认隐藏，看任务管理器）。
- [ ] **Step 5:** 提交 `chore: scaffold tauri app and sidecar fetch`。

### Task 2: 纯逻辑模块（env / settings 接管 / 登录检测 / 配置）

**Files:**
- Create: `src-tauri/src/env_builder.rs`、`src-tauri/src/claude_settings.rs`、`src-tauri/src/auth_status.rs`、`src-tauri/src/config.rs`（测试写在各文件的 `#[cfg(test)]` 里）

**Interfaces（Produces）:**
```rust
// config.rs
#[derive(Serialize, Deserialize, Clone)]
pub struct AppConfig { pub port: u16, pub outbound_proxy: Option<String>, pub codex_home: Option<PathBuf>,
  pub claude_dir: Option<PathBuf>, pub autostart: bool, pub takeover: bool }        // 默认值 18765 / None / None / None / true / true
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct TakeoverRecord { pub base_url: Option<String>, pub models: Vec<String>,
  pub prev_model: Option<Option<String>>, pub prev_env_model: Option<Option<String>> }
pub fn load_json<T: DeserializeOwned + Default>(path: &Path) -> T;
pub fn save_json<T: Serialize>(path: &Path, v: &T) -> io::Result<()>;   // 原子写：先写 .tmp 再 rename

// env_builder.rs
pub fn build_proxy_env(base: &HashMap<String, String>, home: &Path, cfg: &AppConfig) -> HashMap<String, String>;

// claude_settings.rs
pub fn read_settings(path: &Path) -> Result<Value, SettingsError>;      // 文件不存在→空对象；剥 BOM；解析失败→Err(Invalid)
pub fn write_settings(path: &Path, v: &Value) -> io::Result<()>;        // 创建父目录；2 空格缩进；原子写
pub fn apply_takeover(doc: &mut Value, rec: &mut TakeoverRecord, base_url: &str, models: &[String]);
pub fn revoke_takeover(doc: &mut Value, rec: &mut TakeoverRecord) -> Vec<String>;  // 返回警告文案
pub fn is_taken_over(doc: &Value, rec: &TakeoverRecord) -> bool;

// auth_status.rs
pub enum AuthState { Ok { expires_at: i64 }, ExpiredRefreshable, Missing(String) }
pub fn auth_file(cfg_codex_home: Option<&Path>, env: &HashMap<String,String>, home: &Path) -> PathBuf;
pub fn inspect_auth(path: &Path, now_unix: i64) -> AuthState;
```

- [ ] **Step 1: 写失败测试**
  - `env_strips_and_sets`：base 里放 `CODEX_CI=1`、`codex_home=x`、`CCP_PORT=1`、`ANTHROPIC_BASE_URL=u`、`https_proxy=p`、`PATH=a`，`cfg.outbound_proxy=Some("http://127.0.0.1:7890")`，home=`C:\Users\张 三`。断言：结果里没有前四个 key 和 `https_proxy`；`PATH=a`；`HOME=C:\Users\张 三`；`HTTP_PROXY`、`HTTPS_PROXY` 都等于 7890；`NO_PROXY=localhost,127.0.0.1,::1`；`CCP_LOG_STDERR=1`；没有 `CODEX_HOME`。
  - `env_no_outbound`：`outbound_proxy=None` 时，四个代理变量一个都没有。
  - `takeover_roundtrip_preserves_user_keys`：doc 为 `{"env":{"HTTPS_PROXY":"x"},"hooks":{"a":1},"modelPicker":{"options":[{"model":"mine","label":"Mine"}]},"model":"opus"}`。先 `apply_takeover(base_url="http://127.0.0.1:18765", models=["gpt-a","gpt-b"])`，断言 options 依次为 `mine, gpt-a, gpt-b`，label 是 `Codex gpt-a`；再把 `doc["model"]="gpt-a"`，然后 `revoke_takeover`，断言 doc 与最初完全相同（包括 `model=="opus"`）。
  - `apply_twice_replaces_models`：先用 `["gpt-a"]` 接管，再用 `["gpt-c"]` 接管，断言 options 里只剩用户条目和 `gpt-c`。
  - `revoke_keeps_user_modified_base_url`：接管后把 `env.ANTHROPIC_BASE_URL` 改成 `"http://other"`，撤销后它仍是 `"http://other"`，返回 1 条警告。
  - `revoke_on_created_file_leaves_empty_object`：从 `{}` 接管再撤销，结果是 `{}`（没有空的 `env`、`modelPicker`）。
  - `read_settings_bom_ok_and_invalid_err`：带 BOM 的合法 JSON 返回 Ok；`{"a":1,}` 返回 `Err(Invalid)`；文件不存在返回 `Ok({})`。
  - `auth_states`：用临时目录写 auth.json，`access_token` 是手工拼的 JWT（payload 含 `exp`）。验证三种情况：exp 在 now 之后→`Ok`；exp 已过且有 refresh_token→`ExpiredRefreshable`；没有 `tokens`→`Missing`。
  - `auth_file_priority`：cfg 有 codex_home 时用它；否则用 env 里的 `CODEX_HOME`；都没有就用 `home/.codex/auth.json`。
- [ ] **Step 2:** `cargo test --manifest-path src-tauri/Cargo.toml`。预期：全部 FAIL（函数还没实现）。
- [ ] **Step 3:** 按上面的签名实现。`serde_json` 打开 `preserve_order` feature，保证用户 key 的顺序不变；JWT 只做 base64url 解码取 payload，不验签。
- [ ] **Step 4:** 再跑一次 `cargo test`。预期：全部 PASS。
- [ ] **Step 5:** 提交 `feat: env builder, settings takeover, auth inspection`。

### Task 3: 代理监护（spawn / Job / 健康 / 重启 / 日志 / 端口）

**Files:**
- Create: `src-tauri/src/supervisor.rs`、`src-tauri/src/proc_guard.rs`、`src-tauri/src/log_buffer.rs`

**Interfaces:**
- Consumes：`build_proxy_env`、`AppConfig`
- Produces:
```rust
pub enum ProxyState { Starting, Running, Restarting { attempt: u32 }, Failed(String), PortBusy { pid: Option<u32>, is_claude_codex: bool } }
pub enum LogHint { None, Auth401(String), UpstreamNetwork(String) }
pub struct Supervisor { /* Arc 内部状态 */ }
impl Supervisor {
  pub fn new(sidecar: PathBuf, cfg: AppConfig, on_change: impl Fn(ProxyState) + Send + Sync + 'static) -> Self;
  pub fn start(&self); pub fn restart(&self); pub fn stop(&self);           // stop 同步结束子进程
  pub fn kill_port_owner_and_start(&self);
  pub fn state(&self) -> ProxyState; pub fn models(&self) -> Vec<String>;   // 最近一次 /v1/models 的 id
  pub fn logs(&self) -> Vec<String>; pub fn last_hint(&self) -> LogHint;
}
pub fn next_delay(restarts_in_window: usize) -> Option<Duration>;   // 0..4→1,2,5,10,30s；>=5→None
pub fn classify_log_line(line: &str) -> LogHint;                     // 解析上游 JSON 行
// proc_guard.rs
pub fn attach_kill_on_close(child: &std::process::Child);            // Windows: Job+KILL_ON_JOB_CLOSE；Linux: 子进程里 pre_exec 设 PR_SET_PDEATHSIG；mac: no-op（退出时 stop()）
```

- [ ] **Step 1: 写失败测试**（只测纯函数）
  - `backoff_sequence`：`next_delay(0..=4)` 依次是 1/2/5/10/30 秒，`next_delay(5) == None`。
  - `classify_401`：输入 `{"msg":"request_failed","fields":{"status":401,"message":"No Codex credentials. ..."}}`，返回 `Auth401("No Codex credentials. ...")`。
  - `classify_network`：输入 `{"msg":"codex_upstream_request_failed","fields":{"error":"Network error: operation timed out","status":502}}`，返回 `UpstreamNetwork(..)`；`level=info` 的普通请求行返回 `None`。
  - `log_buffer_caps_500`：推入 600 行，`len()==500`，第一行是第 101 行。
- [ ] **Step 2:** 跑 `cargo test supervisor log_buffer`，确认 FAIL。
- [ ] **Step 3:** 实现。要点：
  - 子进程用 `std::process::Command`，先 `env_clear()` 再 `envs(build_proxy_env(...))`；Windows 加 `CREATE_NO_WINDOW`。stdout/stderr 各开一个线程读进 `LogBuffer`，每行都过一遍 `classify_log_line`。
  - 健康检查在独立的 tokio 任务里跑，reqwest client 用 `.no_proxy()`。
  - 启动前先 `TcpListener::bind("127.0.0.1:port")` 试端口；端口被占时用 `netstat2` crate 查 pid，再用 `sysinfo` 看进程名是否为 `claude-codex`，结果写进 `PortBusy`。
  - 60 秒窗口内重启超过 5 次，就停止自动重启，状态设为 `Failed(最后 5 行 stderr)`。
- [ ] **Step 4:** `cargo test` 全 PASS。
- [ ] **Step 5:** 提交 `feat: proxy supervisor with job guard and health checks`。

### Task 4: 应用编排（托盘、窗口、自启、单实例、接管联动、退出撤销）

**Files:**
- Create: `src-tauri/src/controller.rs`、`src-tauri/src/tray.rs`、`src-tauri/src/commands.rs`、`src-tauri/src/shutdown.rs`
- Modify: `src-tauri/src/lib.rs`、`src-tauri/Cargo.toml`、`src-tauri/capabilities/default.json`

**Interfaces:**
- Consumes：Task 2 和 Task 3 的全部接口
- Produces（前端调用）：`get_status() -> StatusDto`、`set_takeover(on: bool)`、`restart_proxy()`、`kill_port_owner()`、`save_config(cfg: AppConfig)`、`open_codex_login()`、`refresh_models()`、`copy_logs() -> String`；事件 `status-changed`（载荷 `StatusDto`）。
  `StatusDto { proxy: ProxyState, port: u16, auth: AuthState, auth_hint: Option<String>, network_hint: Option<String>, takeover: bool, taken_over: bool, models: Vec<String>, config: AppConfig, upstream_version: String, logs_tail: Vec<String> }`

- [ ] **Step 1:** 实现 `controller.rs` 的 `reconcile()`，这是唯一会改 settings.json 的入口，每次状态变化都调用它，规则如下：
  - 代理状态是 `Running`、`cfg.takeover` 为真、且模型列表非空时，`apply_takeover`。
  - 代理状态是 `Failed`，或 `cfg.takeover` 为假时，若当前已接管就 `revoke_takeover`，并弹系统通知「代理不可用，已临时撤销接管，新开的 Claude Code 窗口只用 Claude 模型」。
  - 写盘前后各读一次 settings.json，内容没变就不写。`TakeoverRecord` 存在 app data 目录的 `takeover.json`。
- [ ] **Step 2:** 启动顺序：先读 `TakeoverRecord`，若 settings 里有本客户端写入的残留，先撤销（崩溃恢复）；然后 `supervisor.start()`。监听 `auth.json` 的修改时间（每 5 秒查一次，别引入文件监听库）：登录从 Missing 变为 Ok 时调用 `restart()`。
- [ ] **Step 3:** 托盘：顶部三行状态（不可点击）、打开主窗口、接管 Claude Code（勾选项）、重启代理、开机自启（勾选项）、退出。图标三态：绿（全部正常）、黄（重启中或令牌已过期）、红（失败、端口冲突或登录缺失），图标文件放 `src-tauri/icons/tray-{ok,warn,err}.png`。主窗口 `CloseRequested` 时 `prevent_close()` 并 `hide()`。
- [ ] **Step 4:** 自启和单实例：`tauri-plugin-autostart` 带 `--hidden` 参数启动，此时不显示窗口；`tauri-plugin-single-instance` 在第二个实例启动时 `show()` 加 `set_focus()`。首次运行（app data 里没有 `config.json`）时自动显示主窗口。
- [ ] **Step 5:** `shutdown.rs`：
  - 托盘「退出」：同步执行 `revoke`，再 `stop()`，再 `app.exit(0)`。
  - Windows：用 `SetWindowSubclass` 挂到主窗口 hwnd 上，收到 `WM_ENDSESSION`（wParam=TRUE）时同步执行同样的 revoke 和 stop。
  - Linux / mac：`RunEvent::Exit` 和 SIGTERM（`signal-hook`）都走同一个函数。
- [ ] **Step 6:** `open_codex_login`：Windows 用 `cmd /c start cmd /k codex login`；mac 用 `osascript -e 'tell app "Terminal" to do script "codex login"'`；Linux 依次尝试 `x-terminal-emulator -e`、`gnome-terminal --`、`konsole -e`。找不到 `codex` 时返回错误「未安装 Codex CLI：npm i -g @openai/codex」。
- [ ] **Step 7:** `npm run tauri dev` 手工检查：托盘出现；代理在 18765 监听；`settings.json` 写入了 `ANTHROPIC_BASE_URL` 和模型；从托盘退出后 `settings.json` 恢复，`claude-codex` 进程消失。
- [ ] **Step 8:** 提交 `feat: tray app wiring, takeover reconcile, exit revoke`。

### Task 5: 主窗口 UI

**Files:**
- Create / Modify: `index.html`、`src/main.ts`、`src/styles.css`；`src/icons.ts`（内联 SVG）

- [ ] **Step 1:** 动手前先用 `ui-ux-pro-max` skill 选一套适合「开发者小工具 / 状态面板」的配色和字体（深浅色都要），所有颜色都写成 CSS 变量，配合 `prefers-color-scheme` 切换。
- [ ] **Step 2:** 按 spec 5.5 的线框图实现单页，从上到下：
  - 标题栏加一个总体状态徽章。
  - 三张状态卡（代理 / Codex 登录 / Claude Code），每张卡有状态点、一行说明和一个操作按钮。
  - 告警条：`auth_hint`、`network_hint`、`PortBusy` 时显示，`PortBusy` 带「结束它并接管」按钮。
  - 模型标签云，带刷新按钮。
  - 折叠的设置区：端口、出口代理、Codex 目录、Claude 目录、开机自启；有改动才出现保存按钮。
  - 日志区：等宽字体，最近 200 行，带复制按钮。
  - 页脚一行小字：「改动只对新开的 Claude Code 窗口生效」。
- [ ] **Step 2b: 首次运行引导**（`config.json` 不存在时主窗口显示引导而非主界面，4 步，可后退）：
  1. 环境检测：`codex` CLI 是否在 PATH（显示版本）、auth.json 位置与 `AuthState`、Claude 目录与 settings.json 是否可解析；缺 codex CLI 时给安装命令，登录缺失时给「打开终端运行 codex login」按钮（检测到登录成功自动打勾）。
  2. 出口代理：输入框，预填检测到的系统代理（Windows 读 `HKCU\...\Internet Settings` 的 `ProxyServer`，其它平台读环境变量），可留空；「测试连通」按钮让后端经该代理 `GET https://auth.openai.com` 并显示耗时 / 错误。
  3. 端口与开机自启（默认 18765、开）。
  4. 启动代理并展示可用模型；勾选「接管 Claude Code」（默认勾），点「完成」后写 `config.json`、按选择执行接管，进入主界面。
  后端为此新增命令：`detect_environment() -> EnvReport { codex_cli: Option<String>, auth_path: String, auth: AuthState, claude_settings: String, settings_ok: bool, system_proxy: Option<String> }`、`test_outbound(proxy: Option<String>) -> Result<u64, String>`（毫秒）、`finish_onboarding(cfg: AppConfig)`；`get_status` 的 `StatusDto` 增加 `onboarded: bool`。首次运行时 supervisor 等 `finish_onboarding` 或引导第 4 步才启动。
- [ ] **Step 3:** 数据流：启动时 `get_status`，之后订阅 `status-changed` 并整页重新渲染（数据量很小，不做 diff）。
- [ ] **Step 4:** 用 Edge 无头截图检查浅色和深色两种效果（含引导 4 步各一张）：`npm run build` 后对 `dist/index.html` 截图，用 mock 的 `window.__TAURI__` 注入三种状态（全绿、端口冲突、登录缺失）。逐张查看：文字不截断、状态色分得清、480 宽下不出现横向滚动条。
- [ ] **Step 5:** 提交 `feat: status window ui`。

### Task 6: 打包与 Windows 验收

- [ ] **Step 1:** `npm run tauri build`。预期生成 `src-tauri/target/release/bundle/nsis/Codex Bridge_*_x64-setup.exe`。
- [ ] **Step 2:** 安装后在 Windows 上逐条验收，每条都记下结果：
  1. 开机自启进托盘，窗口不弹出。
  2. 从 Orca 和 PowerShell 各打开再关闭一次终端，代理 pid 都不变。
  3. 在 `/model` 里选 `Codex gpt-6.1-sol` 能正常回答；Claude 模型同样可用。
  4. 端口被一个手动起的 `claude-codex` 占着时启动客户端，界面提示，点按钮后由客户端接管。
  5. 任务管理器里杀掉 `claude-codex`，10 秒内自动恢复。
  6. 把 auth.json 临时改名，登录卡变红并给出 `codex login` 按钮；改回来后自动恢复。
  7. 退出客户端后 `settings.json` 与接管前逐字节一致（`fc /b` 对比），不残留进程。
  8. 把出口代理填成不存在的端口，界面显示「出口网络不可用」，不显示登录错误。
- [ ] **Step 3:** 在 `.github/workflows/build.yml` 加三平台矩阵（windows-latest、macos-13、ubuntu-22.04；x64），只做 build 和上传 artifact。README 写明 mac 需右键打开、GNOME 需要 AppIndicator 扩展。
- [ ] **Step 4:** 提交 `chore: packaging, ci matrix, readme`。
