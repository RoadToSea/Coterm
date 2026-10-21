# 架构与实现状态

本文记录当前已实现的 MVP 和仍保留的边界。长期职责与原则见 [总技术路线](../SPEC.md)；实际模块关系见[当前架构图](current-architecture.md)。插件接口与 JSON-RPC 行协议仍未冻结为 v1。

## 当前已完成

- Tauri 桌面基座可发现、启动和停止插件，创建多个插件实例，并挂载插件自有前端。
- 插件握手注册操作目录和原生 MCP Tools。操作可以是实例级或插件级；基座按 schema 校验输入和输出，按调用来源与 effect 执行审批，维护有界、脱敏的操作账本。
- 桌面和 MCP 均可调用插件级操作。MCP 从工具命名空间确定插件授权域；实例级操作仍需显式 `pluginInstanceId`。插件级操作记录、批准和拒绝使用 `pluginId` 授权域，不借用实例 session。
- Agent commit 默认先准备提案，人工批准使用参数快照；Bypass 是用户显式设置。桌面审批同时处理插件级和实例级提案。
- 操作等待、立即返回、结果查询、待审批/交互取消均已支持。运行中取消会转交插件：插件确认停止才记为 `cancelled`；SSH 目前发送 Ctrl+C，但无法确认远端 shell 已停止，因此记为 `unknown`。
- 固定本机 Streamable HTTP MCP、插件原生 Tools、共享实例、SSH 密码交互和 SSH 输出事件已接通。

## 当前边界

- SSH 运行中取消只能尽力发送 Ctrl+C，最终结果不能确认为停止；状态为 `unknown` 时用户应自行检查远端状态，不能自动重放命令。
- MCP Resources、Prompts、订阅和协议级 cancellation notification 尚未实现。MCP `coterm.cancel_operation` 是 Coterm 操作取消工具。
- 持续数据目前通过有界 JSON 事件推送；正式二进制流、通用 StreamRouter、流量控制 UI 尚未接通。
- 插件包签名、市场、自动更新、秘密物理隔离和安全前端沙箱尚未实现。插件前端属于受信任模块，不是安全边界。
- 操作历史保存在内存，应用重启后不保留。

## 主要接口文件

| 文件 | 职责 |
|---|---|
| [runtime/operation.rs](../../crates/coterm-proto/src/runtime/operation.rs) | 操作描述、来源、effect、scope、审批和记录类型 |
| [runtime/message.rs](../../crates/coterm-proto/src/runtime/message.rs) | 插件进程控制消息、取消消息和执行通知 |
| [plugin_runtime.rs](../../crates/coterm-core/src/plugin_runtime.rs) | 可信调用上下文、操作运行时接口 |
| [plugin_manager.rs](../../crates/coterm-core/src/plugin_manager.rs) | 插件目录、实例和操作路由实现 |
| [operation_history.rs](../../crates/coterm-core/src/operation_history.rs) | 有界操作账本 |
| [server.rs](../../crates/coterm-mcp/src/server.rs) | MCP 工具目录、插件级/实例级路由和公共查询工具 |
| [mcp-http.md](mcp-http.md) | 固定 HTTP 地址、会话和访问边界 |
| [plugin-development.md](plugin-development.md) | 插件作者契约与示例 |

## 验证

工作区检查使用 `cargo test --workspace`、`cargo check --workspace`、`cargo clippy --workspace -- -D warnings`；桌面前端使用 `pnpm build`。SSH 插件的本地 SSH 服务集成测试覆盖审批、共享终端、秘密隔离、插件级操作和运行中取消结果。详细协议边界见 [MCP 实现说明](mcp-stdio.md) 与 [SSH 插件说明](ssh-plugin.md)。
