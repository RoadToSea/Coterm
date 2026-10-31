# Serial 插件文档

Serial 提供通用串口文本与 HEX 收发，支持多实例、人工终端交互和 MCP 调用。

| 文档 | 内容 | 适合读者 |
|---|---|---|
| [接口设计](interfaces.md) | 操作总表、参数、实例配置、结果、事件、审批与容量限制 | 插件开发者、前端开发者、MCP 接入方 |
| [使用说明](usage.md) | 构建启动、人工操作、MCP 示例、结果查询、取消与验证命令 | 使用者与现场验收人员 |

插件业务接口以这里的文档为维护入口；修改实现时同步更新对应说明。宿主的通用协议以 [插件开发](../../../docs/architecture/plugin-development.md)、[运行时通信](../../../docs/architecture/runtime-communication.md) 和 [MCP 接入](../../../docs/architecture/mcp-integration.md) 为准。

## 实现与验收入口

- [操作目录与 MCP 注册](../src/backend.rs)
- [命令列表与回显捕获](../src/command.rs)
- [实例配置、连接与输出缓冲](../src/session.rs)
- [文本与 HEX 编码](../src/payload.rs)
- [插件进程契约测试](../tests/serial.rs)
- [MCP 实施进度与待确认项](../../../docs/progress/mcp.md)
