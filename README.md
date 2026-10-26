# Coterm

人类与 Agent 共享连接实例的插件式通信工具。Tauri 基座负责运行时和公共交互，协议与领域界面由独立插件实现。

当前处于基础闭环收尾阶段，已接通 SSH 与 MCP，串口为初版。完成情况和已知缺口统一维护在 [项目进度](docs/PROGRESS.md)。

## 开发

使用兼容 Tauri v2 的 Rust 工具链；已验证 Node.js 22.22.2、pnpm 11.0.8。测试脚本需要 Node 的类型剥离与 `registerHooks` 支持。

```bash
pnpm install
pnpm plugin:ssh:build
pnpm plugin:serial:build
pnpm tauri:dev
```

插件构建仅生成当前平台的开发包。安装包构建使用 `pnpm tauri:build`，跨平台发布尚待验收。

## 检查

```bash
cargo check --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
pnpm build
pnpm test
```

浏览器测试前需构建桌面与 SSH 前端，并安装 Chrome/Edge/Chromium；可用 `CHROME_BIN` 或 `EDGE_BIN` 指定。串口硬件测试默认 ignored，启用方式见串口说明。

## 目录与文档

- `apps/desktop`：Tauri + React 桌面基座。
- `crates`：共享协议、插件运行时和 MCP Bridge。
- `packages/plugin-sdk`：插件前端契约与通用组件。
- `plugins`：SSH、串口和本地终端骨架。

| 文档 | 用途 |
|---|---|
| [SPEC](docs/SPEC.md) | 架构原则与模块边界 |
| [项目进度](docs/PROGRESS.md) | 已完成、待办与最近验证 |
| [插件开发](docs/architecture/plugin-development.md) | 插件接入与开发流程 |
| [运行时通信](docs/architecture/runtime-communication.md) | 操作、审批、交互与事件契约 |
| [MCP](docs/architecture/mcp-integration.md) | 客户端连接、工具和调用语义 |
| [SSH](docs/architecture/ssh-plugin.md) | SSH 使用与协议限制 |
| [串口](docs/architecture/serial-plugin.md) | 串口使用、限制与硬件验证 |
