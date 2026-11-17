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
npm run tauri build    # 打包安装包（Windows: msi / nsis）
```

目录结构：

```
src/          界面（原生 TypeScript + Vite）
src/locales/  界面多语言
src-tauri/    客户端（Rust / Tauri 2）：托盘、配置接管、代理监护
proxy/        代理（Rust 库），由客户端在进程内启动
docs/         设计文档与实施计划
```

## 第三方代码

`proxy/` 目录的代理源码来自开源项目
[claude-code-with-codex](https://github.com/fcakyon/claude-code-with-codex) v0.8.0，
以 MIT 协议授权，原始许可证见 [`proxy/LICENSE`](proxy/LICENSE)，来源说明见 [`proxy/NOTICE.md`](proxy/NOTICE.md)。
本项目在其基础上删除了 Kimi / Cursor / Grok 后端，并改为进程内库运行。

## 注意

用 ChatGPT 订阅驱动官方客户端以外的工具，可能不符合服务条款，请自行评估风险。
