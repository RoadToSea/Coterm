# 串口插件与基座集成

Serial 的接口设计和使用说明维护在插件自己的 [docs 目录](../../plugins/plugin-serial/docs/README.md)。

| 文档 | 内容 |
|---|---|
| [接口设计](../../plugins/plugin-serial/docs/interfaces.md) | 操作总表、MCP 开放范围、参数、配置、结果、事件及容量限制 |
| [使用说明](../../plugins/plugin-serial/docs/usage.md) | 构建启动、人工操作、MCP 示例、审批查询、取消与硬件验证 |
| [MCP 实施进度](../progress/mcp.md) | 已完成的反馈修复、验证结果和待确认项 |

## 基座与插件的边界

- 基座负责插件生命周期、实例路由、前端挂载、来源识别、审批、公共交互、操作记录和 MCP Bridge。
- Serial 插件负责串口配置与连接、文本和 HEX 收发、控制线、输出缓冲及命令队列；具体设备协议由独立插件实现。
- 前端仅通过基座 API 调用插件；实例级调用由宿主绑定实例，MCP 对外使用公共实例名称，UUID 只在内部路由。
- 插件单独注册 MCP descriptor 和内部操作映射，不能因业务操作存在就自动向 Agent 开放。串口人工单次写入和控制线接口不注册为 MCP 工具。
- Agent 提交经过宿主审批，Bypass 或放行只跳过提交审批；秘密交互、协议认证和交付不确定性仍按运行时规则处理。

通用契约见 [插件开发](plugin-development.md)、[运行时通信](runtime-communication.md) 和 [MCP 接入](mcp-integration.md)。终端公共快捷键见 [终端快捷键](terminal-shortcuts.md)。
