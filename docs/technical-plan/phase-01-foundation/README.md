# 第一阶段：基础闭环

本阶段的目标是验证 Coterm 的最小插件闭环：一个协议插件的后端、可选前端、用户交互和原生 MCP 能力由同一个插件实例关联起来，基座只提供运行时、界面挂载、公共交互和单一 MCP Bridge。阶段方案必须覆盖无 UI 插件和同一插件的多个并行实例。

详细方案暂不在本文件中展开，待确定第一阶段的具体协议和交付边界后补充。

插件作者可先阅读 [插件开发者指南](../../architecture/plugin-development.md)。本阶段的接入规范冻结与验证应覆盖指南中的生命周期、并发、交互、流控和多实例验收要求；指南中的示例目前不是已发布 SDK。

建议后续拆分为：

- `01-scope-and-acceptance.md`：范围、非目标和验收标准
- `02-plugin-instance.md`：插件实例、后端进程和前端模块的关联
- `03-runtime-ipc.md`：基座与插件后端的生命周期和事件通道
- `04-plugin-frontend.md`：前端挂载、PluginAPI 和通用组件使用
- `05-user-interaction.md`：用户输入、密码、确认、取消和接管
- `06-mcp-bridge.md`：原生 MCP Server 注册、路由和生命周期关联
- `07-protocol-plugin.md`：第一种协议插件的边界与实现验证
- `08-test-matrix.md`：集成测试、故障注入和验收矩阵
