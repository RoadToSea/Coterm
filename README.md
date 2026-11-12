# Coterm

面向人类与 Agent 协同的终端交互基座。支持 SSH、串口（Serial）、本地终端、Telnet 与 Raw TCP，以内置驱动提供终端连接、字节调试与 Agent 写操作审批。底层连续信号采集、波形分析与数学计算划归专用的虚拟仪器上位机（如 Lissio）。

五个驱动均以 Rust 库链接到桌面主进程，前端直接嵌入应用，无需旁路 driver.exe。当前主要验收平台为 Windows；已验证范围和未解决问题见[状态](docs/status.md)。

## 开发

使用兼容 Tauri v2 的 Rust 工具链，Node.js 22.22.2、pnpm 11.0.8。完整构建、测试、硬件测速与发布步骤见[开发与验收](docs/development.md)。

```bash
pnpm install --frozen-lockfile
pnpm drivers:build
pnpm tauri:dev
```

Windows 安装包：`pnpm tauri:build --bundles nsis`。创建或恢复实例后显式连接，不重放命令。Agent 写操作复用实例审批，密码通过桌面交互提供。工作区与保存项配置在 Windows 加密保存，数据历史仅驻留有界内存。

## 目录与文档

- `apps/desktop`：Tauri + React 桌面基座。
- `crates/coterm-core`：共享数据模型、实例运行时、审批、操作记录和终端工具。
- `crates/coterm-driver`：SSH、串口、本地终端、Telnet 和 Raw TCP 的内部模块，共用一个 Rust 库。
- `crates/coterm-mcp`：MCP 工具映射、Bridge 与 Stdio 接入。
- `apps/shared`：桌面与内置驱动共用的前端类型和交互组件。
- `scripts`：构建、打包和本机验收脚本。
- `tests`：跨模块测试、测试夹具与浏览器验收；布局截图输出到忽略目录 `.agent-debug/layout/`。
- `docs`：集中维护模块、需求、交互、代码架构及验收。

| 文档 | 用途 |
|---|---|
| [文档入口](docs/README.md) | 按五步阅读和维护 |
| [模块架构](docs/01-modules.md) | 整体职责划分 |
| [模块需求](docs/02-requirements/) | 各模块细致需求 |
| [公共需求](docs/03-common-requirements.md) | 共同约束 |
| [模块交互](docs/04-module-interactions.md) | 数据所有权与交接 |
| [代码架构](docs/05-code-architecture.md) | 整体依赖、公共复用、内部实现 |
| [接口参考](docs/reference/interfaces.md) | 当前 MCP 工具与内部契约 |
| [状态](docs/status.md) | 有效验收与未解决问题 |

调试产物只放 `.agent-debug/<任务名>/`；结束后按 [AGENTS.md](AGENTS.md) 调用 `pnpm debug:clean` 清理。
