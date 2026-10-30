# 串口 MCP 未解决反馈

核对日期：2026-10-30。原始使用环境：Windows 11、Antigravity CLI、Coterm Desktop 0.1.0、COM1 ↔ COM2 虚拟串口对（115200 8N1）。

## 1. 只暴露工具 arguments 的客户端无法设置等待元信息

**使用现象**：Agent 按初始化指引设置 `coterm/waitTimeoutMs`，但将 `_meta` 放入了工具的 `arguments`，调用被拒绝：

```text
operation_args_invalid: Operation arguments do not match the registered schema
```

正确的 `_meta` 位于 `tools/call` 的 `params` 下，与 `name`、`arguments` 同级。当前初始化指引已经写明此位置，但原反馈涉及的客户端只向模型暴露工具参数；这类客户端中，Agent 仍无法直接控制该元信息，把它放入 `arguments` 也仍会被严格 schema 拒绝。

**待完善**：补充这类客户端的调用指引；明确无法设置协议元信息时如何使用默认等待、pending 收据与后续查询。若要提供模型可直接设置的等待选项，应由基座统一设计，避免混入串口领域参数或放宽未知字段校验。

当前契约见 [MCP 等待、结果与取消](../architecture/mcp-integration.md#等待结果与取消)；实现见 [MCP Server](../../crates/coterm-mcp/src/server.rs) 的 `initialize_result` 和 `call_tool`。

## 2. Antigravity 导出的工具参数显示为 null，兼容性尚未确认

**使用现象**：Antigravity 本地导出的工具描述中，`coterm-plugin-serial__serial.command` 显示：

```json
{
  "name": "coterm-plugin-serial__serial.command",
  "parameters": null
}
```

Agent 无法从该描述获知 `commands`、`lineEnding`、`cursor` 等字段的类型、枚举和必填要求，只能查源码或试错。

**核对结果**：当前串口工具注册提供 `inputSchema`，基座 `tools/list` 也保留该字段。因此现有证据不足以认定是服务端未声明 schema；问题可能出在客户端转换、导出或缓存环节，尚需使用当前构建重新验证。

**待完善**：对比同一次连接的 `tools/list` 原始响应与客户端最终暴露给模型的参数定义，检查文本、HEX、读取及插件级工具；定位 `inputSchema` 到客户端 `parameters` 转换中丢失字段的位置。

接口约定见 [串口操作与 MCP](../architecture/serial-plugin.md#操作与-mcp)；实现见 [串口工具注册](../../plugins/plugin-serial/src/backend.rs) 的 `mcp_tools` 和 [MCP Server](../../crates/coterm-mcp/src/server.rs) 的 `tools_list`。
