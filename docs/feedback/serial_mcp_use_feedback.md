# Coterm Serial MCP 使用反馈与痛点总结

- **测试场景**：AI Agent 通过 Coterm Serial MCP 插件连接 RISC-V Linux 开发板 (`COM6@115200`)，完成登录控制台、探测系统信息及配置 `wpa_supplicant` 无线网络。
- **记录时间**：2026-10-31

---

### 1. 工具 Schema 缺失 (`parameters: null`)
- **场景**：Agent 初始化加载 MCP 工具列表并准备调用参数。
- **现象**：`coterm-plugin-serial__serial.*.json` 中的 `parameters` 全为 `null`，没有字段类型、必选参数和描述，Agent 无法从协议层得知该传什么参数，完全依赖盲猜或翻看源码。
- **建议**：导出标准的 JSON Schema (`properties`, `required`, `type`, `description`)。

- **待确认**：服务端已通过真实插件进程测试验证 tools/list 中的标准 inputSchema，仍需核对现场连接的原始响应与客户端 parameters 导出转换。其余已解决项移至 [MCP 进度](../progress/mcp.md)，稳定行为见 [串口插件](../architecture/serial-plugin.md) 与 [MCP 接入](../architecture/mcp-integration.md)。
