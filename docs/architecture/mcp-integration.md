# MCP 接入

## 连接

保持 Coterm 桌面运行，在支持 Streamable HTTP 的客户端添加：

```text
http://127.0.0.1:23517/mcp
```

无需 Token 或转接程序；桌面 MCP 面板可复制地址。重启后地址不变，客户端重新初始化。端口被占用时显示错误，不自动换端口或终止其他进程。

只监听 IPv4 回环，校验 Host/Origin，不开放任意 CORS。该模式不区分本机进程或系统用户，不用于远程访问或多用户隔离。

## 工具与路由

| 工具 | 用途 |
|---|---|
| `coterm.list_instances` | 获取桌面已创建的实例及 ID |
| `coterm.get_operation` | 查询审批、交互和执行结果 |
| `coterm.cancel_operation` | 请求取消，结果依赖插件确认 |
| `<pluginId>__<toolName>` | 插件显式注册的原生 Tool，例如 `coterm-plugin-ssh__ssh.command` |

Bridge 保留插件参数和原生 `CallToolResult`，仅处理命名空间、目标路由及操作元信息。不会自动把整个内部操作目录公开为 MCP。

- 实例级工具的 schema 必须要求字符串 `pluginInstanceId`，与桌面共享同一连接。
- 插件级工具不绑定实例；插件授权域由已注册工具名称确定。
- 查询/取消公共工具使用 `pluginInstanceId` 或 `pluginId` 定位记录授权域。
- 所有外部调用固定为 Agent 来源；Agent 不能自行批准、伪造来源或开启 Bypass。
- 工具结果的机器可读副本是 `structuredContent`；`content[].text` 是简短摘要，不再完整重复负载。`_meta["coterm/operation"]` 只含操作收据，`output` 已替换为大小标记，避免同一结果在响应里出现两次。
- Agent commit 默认经过人工审批。Bypass 只跳过提交审批，仍需协议认证及秘密输入。
- 放行（allowlist）在桌面「Agent approval」面板按操作名显式添加，保存在加密工作区。它同样只跳过审批：effect、插件校验、秘密交互和来源限制都不变，Agent 不能放行、不能自批。
- `effect=read` 且不涉及敏感输入的操作不需要审批；它们只观察状态，不能修改远端。

SSH 认证步骤和工具边界见 [SSH](ssh-plugin.md)，串口可用工具见 [串口](serial-plugin.md)。

## 等待、结果与取消

默认最多等待 60 秒，覆盖审批、秘密交互及执行。调用完成直接返回插件结果，`_meta["coterm/operation"]` 附操作收据。立即返回或等待到期时返回 pending，最终结果可通过查询记录的 `output` 获取。

通过调用元信息设置等待上限，不把它混入领域参数：

```json
{
  "name": "coterm-plugin-ssh__ssh.command",
  "arguments": {"pluginInstanceId": "instance-id", "text": "pwd"},
  "_meta": {"coterm/waitTimeoutMs": 0}
}
```

`0` 表示立即返回，`1..300000` 指定毫秒上限。等待超时、客户端断开或删除 HTTP 会话均不代表底层操作取消，也不触发重放。取消仅在插件确认未提交或已停止时成功，否则可能为 unknown。

## HTTP 契约

- `POST /mcp` 接收单条 JSON-RPC；Accept 包含 JSON 与 SSE。请求返回 JSON，通知/客户端响应返回空 202，不支持 batch。
- `initialize` 不带旧会话 ID；从响应取得 `Mcp-Session-Id`，之后携带会话 ID 与协商的 `MCP-Protocol-Version`。
- 当前支持 `2025-06-18`、`2025-03-26`；初始化完成通知后才能调用工具。
- `GET /mcp` 提供可选 SSE 目录变化通知，每会话最多一条流；无事件回放或断点续传，不传终端输出。
- `DELETE /mcp` 只关闭客户端会话；过期或重启前的会话返回 404，应重新初始化。
- 最多 8 会话、每会话 16 个在途请求、全局 128 个 HTTP 请求；空闲 30 分钟过期。SSE 流处于等待状态不算客户端活动，不会续期，否则挂起的客户端会把名额占满；过期会话的下一条请求返回 404。会话满时返回 429，附 `mcp_sessions_full`、上限、当前会话的客户端名称与空闲时长、`Retry-After: 30` 和两条恢复路径：客户端发送 `DELETE /mcp`，或在 Coterm 的 MCP 面板按前缀关闭会话。面板只显示 ID 前缀，不公开完整会话 ID。
- 请求与 JSON 结果各不超过 1 MiB，读取请求体限时 10 秒；超限明确失败，不无限排队。

实现与测试位于 [coterm-mcp](../../crates/coterm-mcp)。Resources、Prompts、订阅、进度和 MCP cancellation notification 尚未实现；`coterm.cancel_operation` 是操作工具。

旧 `coterm-mcp-stdio` 仅保留作开发回归，通过 `COTERM_MCP_ADDR`/`COTERM_MCP_TOKEN` 连接测试私有服务，不自动转接当前桌面 HTTP 入口。内部交互和事件见 [运行时通信](runtime-communication.md)。
