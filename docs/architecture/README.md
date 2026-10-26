# Coterm 架构专题

这些文档解释总技术路线中的架构边界。它们描述基座、插件后端、插件前端和 MCP 的关系，不绑定某个具体通信协议。

**插件开发者从 [插件开发者指南](plugin-development.md) 开始即可。** 该文档集中维护插件职责、领域接入语义、交互流程和验收要求；运行时线缆、帧边界和背压规则见[基座与插件后端运行时通信](runtime-communication.md)。具体消息编码和 SDK 尚待冻结，不应将草案示例当成已实现接口。

插件前端共享契约和可选基座 UI 组件位于 [`packages/plugin-sdk`](../../packages/plugin-sdk)。桌面应用自身的 UI 不属于插件 SDK。

- [插件运行时与前端模块](plugin-runtime.md)
- [基座与插件后端运行时通信](runtime-communication.md)
- [插件原生 MCP 集成](mcp-integration.md)
- [固定 MCP 连接（Streamable HTTP，仅填写 URL）](mcp-http.md)
- [MCP Tools、后端秘密输入与旧 stdio 适配器](mcp-stdio.md)
- [插件开发者指南](plugin-development.md)
- [接口文件与评审入口](interface-review.md)
- [当前已实现架构图](current-architecture.md)
- [项目进度与缺口评估](../PROGRESS.md)
- [SSH 插件 MVP 与运行方式](ssh-plugin.md)
- [串口插件初版与当前限制](serial-plugin.md)
- [WS-13 集成与回归验收记录](ws13-integration-acceptance.md)
