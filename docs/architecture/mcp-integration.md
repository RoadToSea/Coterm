# MCP 接入

## 连接

MCP 只有一个对外入口：本地 Stdio 子进程。桌面前端通过 Tauri Host API 和插件事件工作，不经过 MCP；对外不提供 HTTP 端点。

适配器是桌面可执行文件自身，以 `mcp stdio` 启动，把 stdin/stdout 的 MCP JSON-RPC 转发给桌面的私有本机 IPC Bridge。该模式不创建窗口、插件运行时或网络监听，stdout 只写 MCP 帧，诊断只写 stderr。参数必须完全匹配，其他启动方式仍是普通桌面。

桌面 MCP 面板负责交付配置：

1. 用户点「启用」，桌面在随机回环端口启动私有 Bridge 并签发本次启动有效的 Token；停用或退出即释放，旧 Token 立即失效。
2. 面板给出可直接粘贴的完整客户端配置：

```json
{
  "mcpServers": {
    "coterm": {
      "command": "<Coterm 安装目录>\coterm-desktop.exe",
      "args": ["mcp", "stdio"],
      "env": {
        "COTERM_MCP_ADDR": "127.0.0.1:<bridge-port>",
        "COTERM_MCP_TOKEN": "<本次启动签发>"
      }
    }
  }
}
```

`command` 取当前可执行文件的绝对路径，不搜索 PATH；`COTERM_MCP_ADDR` 与 `COTERM_MCP_TOKEN` 也可由用户自行设置，适配器只认这两个环境变量，不读取本机配置文件。运行时 Token 不匹配或端口不监听时，适配器在 stderr 报错退出，stdout 保持为空，客户端可据此显示启动失败而不是空目录。

安全边界：桥只监听 IPv4 回环、随机端口，并用定长比较校验 Token；Token 只经桌面私有 Tauri 命令返回面板，不写盘、不进事件、日志、MCP 返回值或插件前端，配置中的 Token 按秘密处理。未启用时面板不显示配置。桌面必须保持运行，适配器不启动插件、不接触前端。内部交互和事件见 [运行时通信](runtime-communication.md)。

## 工具与路由

| 工具 | 用途 |
|---|---|
| `coterm.list_instances` | 获取桌面已创建的实例名称、插件和状态；内部 UUID 不对 MCP 暴露 |
| `coterm.get_operation` | 查询审批、交互和执行结果 |
| `coterm.cancel_operation` | 请求取消，结果依赖插件确认 |
| `<pluginId>__<toolName>` | 插件显式注册的原生 Tool，例如 `coterm-plugin-ssh__ssh.command` |

Bridge 保留插件参数和原生 `CallToolResult`，仅处理命名空间、目标路由及操作元信息。不会自动把整个内部操作目录公开为 MCP。

- 实例级工具的 schema 要求字符串 `name`。它对应 `coterm.list_instances` 返回的唯一实例名称；Bridge 在内部解析为 UUID 后再路由，插件不会跨实例搜索。
- 插件级工具不绑定实例；插件授权域由已注册工具名称确定。
- 查询/取消公共工具使用 `name` 或 `pluginId` 定位记录授权域；UUID 只存在于 Bridge 和运行时内部。
- 所有外部调用固定为 Agent 来源；Agent 不能自行批准、伪造来源或开启 Bypass。
- 工具结果的机器可读副本是 `structuredContent`；`content[].text` 是简短摘要，不再完整重复负载。`_meta["coterm/operation"]` 只含操作收据，`output` 已替换为大小标记，避免同一结果在响应里出现两次。
- Agent commit 默认经过人工审批。Bypass 只跳过提交审批，仍需协议认证及秘密输入。
- 放行（allowlist）在桌面「Agent approval」面板按操作名显式添加，保存在加密工作区。它同样只跳过审批：effect、插件校验、秘密交互和来源限制都不变，Agent 不能放行、不能自批。
- `effect=read` 且不涉及敏感输入的操作不需要审批；它们只观察状态，不能修改远端。

SSH 认证步骤和工具边界见 [SSH](ssh-plugin.md)，串口可用工具见 [串口](serial-plugin.md)。

## 等待、结果与取消

默认最多等待 60 秒，覆盖审批、秘密交互及执行。调用完成直接返回插件结果，`_meta["coterm/operation"]` 附操作收据。立即返回或等待到期时返回 pending，最终结果可通过查询记录的 `output` 获取。

通过调用元信息设置等待上限，不把它混入领域参数。`_meta` 位于 JSON-RPC `tools/call` 的 `params` 顶层，与 `name`、`arguments` 同级：

```json
{
  "name": "coterm-plugin-ssh__ssh.command",
  "arguments": {"name": "user@example.com:22", "commands": ["pwd"]},
  "_meta": {"coterm/waitTimeoutMs": 0}
}
```

`0` 表示立即返回，`1..300000` 指定毫秒上限。等待超时或客户端断开均不代表底层操作取消，也不触发重放。取消仅在插件确认未提交或已停止时成功，否则可能为 unknown。

## 适配器传输

- 每会话走一条新建的 TCP 连接：先发一帧 `{"token":"…"}`，桌面回 `{"authenticated":true}` 后才开始转发 MCP 消息；认证失败直接断开。
- 帧为单行 JSON，请求与响应各不超过 1 MiB；超限明确失败，不无限排队。
- 每连接最多 16 个在途工具调用，最多 8 个并发连接；超出返回 JSON-RPC 错误而不是排队。
- 支持 `2025-06-18`、`2025-03-26`、`2024-11-05`；`initialize` 后必须发送 `notifications/initialized` 才能调用工具。
- 目录变化通过 `notifications/tools/list_changed` 通知，适配器逐帧透传，不做事件回放。
- stdio 断开即结束对应桥连接；这不代表底层操作取消，也不会重放。

实现与测试位于 [coterm-mcp](../../crates/coterm-mcp)。Resources、Prompts、订阅、进度和 MCP cancellation notification 尚未实现；`coterm.cancel_operation` 是操作工具。仓库内的 `coterm-mcp-stdio` 二进制保留为开发回归入口，与桌面 `mcp stdio` 共用同一实现。
