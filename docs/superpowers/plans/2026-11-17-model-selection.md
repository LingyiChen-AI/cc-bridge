# Model Selection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在现有模型列表中选择要注入 Claude Code 的 Codex 模型，默认最新六个，并加宽窗口。

**Architecture:** `AppConfig.selected_models: Option<Vec<String>>` 以 `None` 表示旧安装及首次启动的默认前六个，`Some` 表示用户显式选择。`Controller` 计算可用的有效选择并交给现有接管逻辑；前端复选框写回配置。窗口配置改为 600px。

**Tech Stack:** Rust, Tauri 2, TypeScript, CSS

**Spec:** `docs/superpowers/specs/2026-11-17-model-selection-design.md`

## Global Constraints
- 未配置选项默认取当前有序模型前 6 个；显式空列表表示不接管。
- 不改动用户已有的非本客户端模型选项和代理暴露的模型集合。
- 不强制修改当前使用的模型。
- 不提交 Git commit，除非用户另行要求。

## Review Focus
- 空选择：从已有接管状态撤销注入，不能只跳过更新。
- 刷新后模型消失：从 `/model` 中删除失效项，不能添加未选新模型。
- 老版本配置缺少字段：默认选六个而非零个或全部。
- 从选中切为取消、再选回：恢复逻辑不能覆盖先前模型配置。
- 状态定时更新时正在操作列表：复选框状态不得被过期写入覆盖。

---

### Task 1: 后端模型选择与接管

**Files:**
- Modify: `src-tauri/src/config.rs`
- Modify: `src-tauri/src/controller.rs`
- Test: `src-tauri/src/config.rs`, `src-tauri/src/claude_settings.rs`

**Interfaces:**
- Produces: `AppConfig.selected_models: Option<Vec<String>>`; `AppConfig::effective_models(&self, available: &[String]) -> Vec<String>`; `Controller::set_selected_models(&self, selected: Vec<String>)`.

- [ ] 先写单测：缺字段反序列化默认前 6；显式空选、失效模型和更新后的模型顺序均符合设计。
- [ ] 运行 `cargo test --manifest-path src-tauri/Cargo.toml` 确认新测试先失败。
- [ ] 实现配置有效选择与单独的保存命令；对选择进行可用模型校验和去重；接管同步只写有效模型，空选时撤销。
- [ ] 运行 Rust 单测，确认通过。

### Task 2: 主界面模型勾选及窗口加宽

**Files:**
- Modify: `src/main.ts`
- Modify: `src/api.ts`
- Modify: `src/mock.ts`
- Modify: `src/styles.css`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/tauri.conf.json`

**Interfaces:**
- Consumes: `selected_models` 配置字段和 `set_selected_models(selected: string[])` Tauri 命令。

- [ ] 在模型列表中渲染复选框，首次默认展示 6 行，显示选中数；修改后保存并立即同步接管配置，保留展开状态。
- [ ] 更新 mock 与引导配置，避免配置缺字段造成不一致。
- [ ] 将窗口宽改到 600px，调整模型行的交互和溢出样式。
- [ ] 运行 `npm run build` 和 `cargo test --manifest-path src-tauri/Cargo.toml`，核对新旧配置、空选与刷新路径。
