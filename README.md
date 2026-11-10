# Coterm

面向人类与 Agent 协同的终端交互基座。专注 SSH、串口（Serial）与本地终端，以内置驱动提供终端连接与 Agent 写操作审批。底层连续信号采集、波形分析与数学计算划归专用的虚拟仪器上位机（如 Lissio）。

当前目标为 Windows 上运行可信本地驱动的基座首版；SSH、串口与跨平台发布独立迭代。完成情况和已知缺口统一维护在 [项目进度](docs/progress/project.md)。

## 开发

使用兼容 Tauri v2 的 Rust 工具链；已验证 Node.js 22.22.2、pnpm 11.0.8。测试脚本需要 Node 的类型剥离与 `registerHooks` 支持。

```bash
pnpm install --frozen-lockfile
pnpm drivers:build
pnpm tauri:dev
```

Windows 安装包：`pnpm tauri:build --bundles nsis`。核心终端驱动随应用直接分发，打开应用即可直接创建 SSH 或串口会话，开箱即用。开发版读取仓库 `target/bundled-drivers/{ssh,serial,local}`。

**设置 → 通用 → 语言 / Language** 支持简体中文、English，首次跟随系统语言，选择后自动保存；切换语言保留已打开的实例与 SSH 连接。

Windows 工作区自动加密保存；下次启动点击 **Restore workspace** 重建实例，再主动连接。历史仅保留本次启动，Bypass 每次重置为关闭。

## 检查

```bash
cargo check --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
pnpm build
pnpm test
```

Windows CI 执行检查、浏览器与真实安装验收，并上传 NSIS 安装包；`v*` 标签发布安装产物。跨平台安装另行验收。

浏览器测试前需构建桌面与三个驱动前端，并安装 Chrome/Edge/Chromium；可用 `CHROME_BIN` 或 `EDGE_BIN` 指定。串口硬件测试默认 ignored，启用方式见串口说明。

临时调试内容统一放在本地忽略目录 `.agent-debug/<任务名>/`。`pnpm debug:clean --dry-run` 预览待清理内容，`pnpm debug:clean` 清空内容并保留目录；执行前停止调试进程，确认其他任务未使用该目录。脚本不接受其他目录参数。

## 目录与文档

- `apps/desktop`：Tauri + React 桌面基座。
- `crates`：共享协议、运行时和 MCP Bridge。
- `packages/driver-sdk`：驱动前端契约与通用组件。
- `drivers`：SSH、串口和本地终端驱动。
- `scripts`：构建、打包和本机验收脚本。
- `tests`：跨模块测试、测试夹具与浏览器验收；布局截图输出到忽略目录 `.agent-debug/layout/`。
- `docs`：技术规范、架构、方案、反馈、进度与发布文档。

| 文档 | 用途 |
|---|---|
| [SPEC](docs/SPEC.md) | 架构原则与模块边界 |
| [终端架构](docs/design/terminal-focus-and-in-tree-architecture.md) | 终端聚焦与内置驱动架构决策 |
| [项目进度](docs/progress/project.md) | 已完成、待办与最近验证 |
| [Design](docs/design/README.md) | 复杂业务实现前的方案与验收条件 |
| [Feedback](docs/feedback/README.md) | 外部使用反馈与现场观察 |
| [Release](docs/release/README.md) | 构建、发行与发布前验收 |
| [运行时通信](docs/architecture/runtime-communication.md) | 操作、审批、交互与事件契约 |
| [MCP](docs/architecture/mcp-integration.md) | 客户端连接、工具和调用语义 |
| [SSH](docs/architecture/ssh-driver.md) | SSH 使用与协议限制 |
| [串口](docs/architecture/serial-driver.md) | 串口使用、限制与硬件验证 |
