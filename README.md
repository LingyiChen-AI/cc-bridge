# CC Bridge

托盘常驻的小工具：在本机运行一个代理，让 Claude Code 在同一个 `/model` 列表里同时使用
**Claude 订阅**（Claude 模型）和 **ChatGPT 订阅**（Codex 模型），按模型名自动分流。

- 点窗口关闭只会缩到托盘；开机自启；单实例
- 代理在客户端进程内运行（单个 exe），崩溃自动重启，端口冲突可一键接管
- 由客户端管理 `~/.claude/settings.json`：开启接管时写入代理地址和勾选的 Codex 模型，退出 / 关机时自动恢复
- Codex 登录状态检测（401 等问题直接给出原因）、出口代理配置与连通测试
- 首次运行引导；10 种界面语言（跟随系统）

## 开发

```bash
npm install
npm run tauri dev      # 开发模式：端口 18766、只写沙盒配置，不影响全局 Claude Code
npm run tauri build    # 本地打包；需要更新签名私钥，见下文
```

本地打包会顺带生成更新包签名，需要先设置私钥（私钥在 `~/.tauri/cc-bridge.key`，不入库）：

```bash
export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/cc-bridge.key)"
```

## 发版与自动更新

```bash
git tag v0.2.0 && git push origin v0.2.0
```

推送 `v*` tag 后，GitHub Actions（`.github/workflows/release.yml`）会：

1. 按 tag 设置版本号，并行打包 Windows、macOS Apple 芯片、macOS Intel 三个版本；
2. 上传到同一个草稿 Release，并生成更新清单 `latest.json`（带签名）；
3. 全部成功后正式发布。master 每次推送会由 `build-cache.yml` 预热编译缓存，发版时依赖不用从头编译。

客户端（正式版）启动 30 秒后检查一次更新，之后每 6 小时检查一次（走设置里的出口代理）。
发现新版本时主窗口顶部出现提示并发一次系统通知，点「立即更新」后先撤销接管、停止代理，再安装并重启。
设置面板里可以看到当前版本号和「检查更新」按钮。

仓库 Secrets 需要 `TAURI_SIGNING_PRIVATE_KEY`（私钥文件内容）。私钥丢失后已安装的客户端将无法再验证更新，务必备份。

目录结构：

```
src/          界面（原生 TypeScript + Vite）
src/locales/  界面多语言
src-tauri/    后端（Rust / Tauri 2）：托盘、配置接管、代理监护；src/proxy/ 是代理本身（模块）
docs/         设计文档与实施计划
```

## 第三方代码

`src-tauri/src/proxy/` 目录的代理源码来自开源项目
[claude-code-with-codex](https://github.com/fcakyon/claude-code-with-codex) v0.8.0，
以 MIT 协议授权，原始许可证见 [`src-tauri/src/proxy/LICENSE`](src-tauri/src/proxy/LICENSE)，来源说明见 [`src-tauri/src/proxy/NOTICE.md`](src-tauri/src/proxy/NOTICE.md)。
本项目在其基础上删除了 Kimi / Cursor / Grok 后端，并作为后端的一个模块直接编进客户端。

## 注意

用 ChatGPT 订阅驱动官方客户端以外的工具，可能不符合服务条款，请自行评估风险。
