# 架构与实现状态

本文记录当前已实现的 MVP 和仍保留的边界。长期职责与原则见 [总技术路线](../SPEC.md)；实际模块关系见[当前架构图](current-architecture.md)。插件接口与 JSON-RPC 行协议仍未冻结为 v1。

## 当前已完成

- Tauri 桌面基座可发现、启动和停止插件，创建多个插件实例，并挂载插件自有前端。
- 插件握手注册操作目录和原生 MCP Tools。操作可以是实例级或插件级；基座按 schema 校验输入和输出，按调用来源与 effect 执行审批，维护有界、脱敏的操作账本。
- 桌面和 MCP 均可调用插件级操作。MCP 从工具命名空间确定插件授权域；实例级操作仍需显式 `pluginInstanceId`。插件级操作记录、批准和拒绝使用 `pluginId` 授权域，不借用实例 session。
- 基座向插件实例 API 提供可选通用块列表组件，支持编号、来源标签、退出码和多选；块边界与数据由插件定义，选择结果由插件决定如何用于自身 MCP 上下文。
- Agent commit 默认先准备提案，人工批准使用参数快照；Bypass 是用户显式设置。桌面审批同时处理插件级和实例级提案。
- 审批面板横向铺满工作区底部，顶部圆角接到两侧边框、底部贴合状态栏，一行可编辑命令或紧凑 JSON、一行通过/否决按钮，从底边向上滑入。无模态遮罩、不抢焦点，面板外保持可操作；焦点在面板内时 Enter 通过、Esc 否决，输入法确认和按键重复不会触发通过。单字符串参数直接编辑并保留其他路由参数，多字段参数编辑完整 JSON；提交仍走基座原有快照校验和目标绑定。
- 操作等待、立即返回、结果查询、待审批/交互取消均已支持。运行中取消会转交插件：插件确认停止才记为 `cancelled`；SSH 目前发送 Ctrl+C，但无法确认远端 shell 已停止，因此记为 `unknown`。
- 固定本机 Streamable HTTP MCP、插件原生 Tools、共享实例、SSH 密码交互和 SSH 输出事件已接通。
- 插件进程的 stdio 传输骨架统一在 `coterm_proto::runtime::transport`：信封编解码、控制行大小上限、读行与 tick 的调度、通知推送串行化都在这一层，插件只实现 `PluginBackend` 的领域逻辑。SSH 与串口共用它，`main.rs` 各自只剩一行 `serve(Backend::default())`。
- 串口插件已接入：端口枚举、连接、原始字节读写、有界回放、审批和 MCP 只读工具，并在真实 CH340 设备上以 115200 8-N-1 验证。详见 [串口插件](serial-plugin.md)。
- 插件发现是容错的：单个插件包加载失败只记录并跳过，不再让整个插件列表失败。

## 当前边界

- SSH 运行中取消只能尽力发送 Ctrl+C，最终结果不能确认为停止；状态为 `unknown` 时用户应自行检查远端状态，不能自动重放命令。
- MCP Resources、Prompts、订阅和协议级 cancellation notification 尚未实现。MCP `coterm.cancel_operation` 是 Coterm 操作取消工具。
- 持续数据目前通过有界 JSON 事件推送；正式二进制流、通用 StreamRouter、流量控制 UI 尚未接通。
- 插件包签名、市场、自动更新、秘密物理隔离和安全前端沙箱尚未实现。插件前端属于受信任模块，不是安全边界。
- 操作历史保存在内存，应用重启后不保留。
- 串口插件不暴露 `serial.write` 给 MCP：原始字节直接驱动硬件，写路径只留给人工界面。`plugin-local` 仍是早期的占位插件，尚未迁移到 `PluginBackend` 与当前 manifest 结构。
- 串口未实现流控、DTR/RTS 控制、断线自动重连和端口热插拔监听；`serialport` crate 为同步接口，因此读写走独立线程。

## 主要接口文件

| 文件 | 职责 |
|---|---|
| [runtime/operation.rs](../../crates/coterm-proto/src/runtime/operation.rs) | 操作描述、来源、effect、scope、审批和记录类型 |
| [runtime/message.rs](../../crates/coterm-proto/src/runtime/message.rs) | 插件进程控制消息、取消消息和执行通知 |
| [runtime/transport.rs](../../crates/coterm-proto/src/runtime/transport.rs) | 插件 stdio 传输骨架：信封、读行、tick、通知推送 |
| [plugin_runtime.rs](../../crates/coterm-core/src/plugin_runtime.rs) | 可信调用上下文、操作运行时接口 |
| [plugin_manager.rs](../../crates/coterm-core/src/plugin_manager.rs) | 插件目录、实例和操作路由实现 |
| [operation_history.rs](../../crates/coterm-core/src/operation_history.rs) | 有界操作账本 |
| [server.rs](../../crates/coterm-mcp/src/server.rs) | MCP 工具目录、插件级/实例级路由和公共查询工具 |
| [mcp-http.md](mcp-http.md) | 固定 HTTP 地址、会话和访问边界 |
| [plugin-development.md](plugin-development.md) | 插件作者契约与示例 |
| [serial-plugin.md](serial-plugin.md) | 串口插件的操作、线程模型、数据限制和实测结果 |

## 验证

工作区检查使用 `cargo test --workspace`、`cargo check --workspace`、`cargo clippy --workspace -- -D warnings`；桌面前端使用 `pnpm build`。SSH 插件的本地 SSH 服务集成测试覆盖审批、共享终端、秘密隔离、插件级操作和运行中取消结果。串口插件覆盖配置校验、回放缺口、写入负载边界、MCP 实例绑定和真实基座加载，并在 CH340 串口上以 115200 实测读写往返。详细协议边界见 [MCP 实现说明](mcp-stdio.md)、[SSH 插件说明](ssh-plugin.md) 与 [串口插件说明](serial-plugin.md)。
