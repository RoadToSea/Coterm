# MCP 接入

## 连接

MCP 只有一个对外入口：本地 Stdio 子进程。桌面前端通过 Tauri Host API 和插件事件工作，不经过 MCP；对外不提供 HTTP 端点。

适配器是桌面可执行文件自身，以 `mcp stdio` 启动，把 stdin/stdout 的 MCP JSON-RPC 转发给桌面的私有本机 IPC Bridge。该模式不创建窗口、插件运行时或网络监听，stdout 只写 MCP 帧，诊断只写 stderr。`mcp stdio` 前缀确定适配器模式，其余选项在适配器中校验，错误选项只向 stderr 报错退出，不会意外启动窗口。

设置中的「MCP」分类提供独立的「MCP 端口」「MCP 连接」「客户端配置」三项；右侧 MCP 分类标题处展示当前「已启动 / 未启动」，读取失败单独显示状态不可用，不在搜索框上方或其他分类显示。端口项只保留输入与保存，连接项负责启动方式与手动启停，客户端配置单独一行提供查看 / 复制入口：

1. 「自启动 / 手动启动」使用各占一半的单选胶囊。默认自启动，随 Coterm 启动在本机 IPv4 回环地址启动私有 Bridge；默认 `127.0.0.1:51234`，端口可在设置中修改。手动模式显示「启动 MCP / 关闭 MCP」按钮；关闭接口或退出程序即释放监听。
2. 面板给出可直接粘贴的完整客户端配置：

```json
{
  "mcpServers": {
    "coterm": {
      "command": "<Coterm 安装目录>\coterm-desktop.exe",
      "args": ["mcp", "stdio"]
    }
  }
}
```

`command` 取当前可执行文件的绝对路径，不搜索 PATH。以上为默认端口的兼容配置；使用自定义端口时，复制出的 `args` 自动包含 `--addr`，例如 `["mcp", "stdio", "--addr", "127.0.0.1:12345"]`。适配器也保留可选 `COTERM_MCP_ADDR` 环境变量。优先级为 `--addr` → 环境变量 → 默认地址；参数不得重复，地址必须是回环地址且带非零端口。无需 Token，不提供 `--token`。桥未启用或端口不可用时，适配器在 stderr 报错退出，stdout 保持为空，客户端可据此显示启动失败而不是空目录。

安全边界：桥只监听 IPv4 回环地址 `127.0.0.1`，用户只能调整端口，不接受局域网连接，也不开放 HTTP 接口。MCP 调用统一标记为 Agent 来源，写操作仍经过基座审批/Bypass 规则、插件校验和协议认证。端口本身不承担身份认证，桌面必须保持运行，适配器不启动插件、不接触前端。内部交互和事件见 [运行时通信](runtime-communication.md)。

### 端口设置与宿主接口

- 「设置 → MCP → MCP 端口」接受 1–65535 的整数，默认 51234；中英文搜索支持 MCP / port / 端口 / 接口。修改草稿不生效，点击保存后生效；显示偏好的重置不修改端口。
- 端口在应用数据目录的 `mcp-settings.v1.json` 持久化，文档为 `{"version":1,"port":51234,"startupMode":"auto"}`；文件保存非敏感端口与启动方式，不包含插件配置或秘密。读取上限 4 KiB，拒绝未知字段、错误版本、非法端口与未知启动方式；旧版只有端口的文件兼容读取并使用自启动。写入使用同目录临时文件及原子替换，Windows 复用现有文件替换辅助函数，其他平台使用同文件系统 rename；运行中保存通过阻塞任务执行，不阻塞 UI 线程。
- 已启用时先保留旧桥、绑定新端口，再保存；两步都成功后才切换并关闭旧连接。占用、权限或保存失败保留原桥、原端口及输入草稿；重复保存同端口不重启桥。停用状态只检查端口可用性并保存，不启动 MCP 服务。重启桌面会读取已保存端口。
- 设置文件损坏或不可读时不偷偷回退开启默认端口；状态返回配置错误，用户保存有效端口后恢复。MCP 客户端连接会在成功换端口时断开，须从「MCP 连接」重新复制配置并重连，不自动重发已提交的工作。
- `mcp_stdio_status`、`set_mcp_stdio_enabled` 及 MCP 设置命令返回 `enabled / command / address / clientConfig / port / startupMode / configurationError`。`port` 在停用时仍返回已配置值，`address / clientConfig` 仅在启用时返回。
- 新宿主命令 `set_mcp_stdio_port` 接收 `{request:{port:12345}}`，返回同一状态结构；仅可信桌面设置调用，不注册为 MCP 工具，不新增插件运行时消息或 Tauri capability。校验和切换在后端串行执行，禁止前端自行指定监听 IP。
- `set_mcp_startup_mode({request:{startupMode:"auto"|"manual"}})` 仅供可信桌面调用。启动方式和端口使用同一设置文件，修改任一项保留另一项；先完成必要的监听及原子保存，再提交运行时状态，失败保留原启动方式和原服务。切换到自启动立即尝试启动；切到手动不关闭正在运行的服务，只改变下次启动行为。手动启动/关闭不修改持久化模式，下次打开 Coterm 保持未启动。此处自启动只指随 Coterm 启动，不是操作系统登录启动。
- 设置中的状态、端口与连接控件共用后端快照及单一操作锁；保存、模式切换和启停期间禁用相关控件，收到成功响应后再更新胶囊和分类标题状态。首次读取失败可重试，关闭设置后忽略迟到响应。客户端配置弹窗只负责查看 / 复制，不再另设启停开关。

## 工具与路由

| 工具 | 用途 |
|---|---|
| `coterm.list_instances` | 获取桌面已创建的实例名称、插件和状态；内部 UUID 不对 MCP 暴露 |
| `coterm.status` | 无参数、只读的桌面 / Bridge 与实例连接快照；区分 ready、已连接和未知 |
| `coterm.get_operation` | 查询审批、交互和执行结果 |
| `coterm.cancel_operation` | 请求取消，结果依赖插件确认 |
| `coterm_<plugin>_<tool>` | 插件显式注册的原生 Tool，例如 `coterm_ssh_exec`、`coterm_serial_command` |

SSH 命令统一由 `coterm_ssh_command` 提交，默认 `mode=terminal`；预计长任务或用户明确要求后台时显式选择 `mode=exec`。读取使用 `coterm_ssh_read_lines`、`coterm_ssh_read_selected_lines` 和 `coterm_ssh_read_dialogue_pairs`，均为后端快照，不要求 Agent 维护游标。xterm 选区由仅 Human 可用的 `ssh.select_lines` 同步；Agent 不能伪造选择。数据结构和界限见 [SSH 驱动](ssh-plugin.md#后端-json-记录与读取)。

工具名映射先去掉插件 ID 的 `coterm-plugin-` 前缀，再去掉原生工具名重复的 `<plugin>.` 前缀，非 ASCII 字母数字统一替换为 `_`，最后添加 `coterm_`。例如 `coterm-plugin-ssh` + `ssh.command` → `coterm_ssh_command`。`tools/list` 只发布新名称；旧 `<pluginId>__<toolName>` 仍作为未列出的兼容别名接受，不重复发布。映射碰撞时目录明确报 `mcp_tool_name_conflict`，调用歧义名称也拒绝，绝不按注册顺序任选插件。内部操作名与进程协议不变。

Bridge 保留插件参数和原生 `CallToolResult`，仅处理命名空间、目标路由及操作元信息。不会自动把整个内部操作目录公开为 MCP。

`tools/list` 保留插件注册提供的 `inputSchema`；客户端导出界面中的 `parameters` 属于客户端转换结果。若客户端本地显示为 `null`，应对比同一连接的原始 `tools/list` 响应与客户端导出值，服务端集成测试会验证 schema 是否已发送。插件工具目录由已启动插件的握手注册；当前 manifest 只标识 MCP provider，不含静态工具 schema，因此未启动插件的工具不会出现在目录中，也没有 MCP 自动恢复工作区实例的操作。

- 实例级工具的公共 schema 中 `name` 可选，显式传入时必须是 `coterm.list_instances` 返回的有效名称。仅目标插件恰有一个确认已连接的实例、无未知状态且快照未截断时，省略 `name` 才能自动路由；第二个未连接实例不妨碍唯一已连接目标。零个 / 多个已连接、未知或超限时提示指定 `name`，不提交原操作。Bridge 兼容 `instance`、`instanceName`、`instance_name`，一个调用只能提供一种名称字段；空值、错误类型、冲突别名和显式无效名称不会退回默认目标。Bridge 仅在内部解析为 UUID，调用方不能传 `pluginInstanceId`。
- 插件级工具不绑定实例；插件授权域由已注册工具名称确定。
- 查询/取消公共工具使用 `name` 或 `pluginId` 定位记录授权域；不能同时传两者。都省略时要求所有插件中恰有一个确认已连接的默认实例且无未知状态，仍验证操作所属插件和实例。实例断开、默认目标改变或存在未知状态时，查询旧操作请显式传原 `name`。UUID 只存在于 Bridge 和运行时内部。
- 所有外部调用固定为 Agent 来源；Agent 不能自行批准、伪造来源或开启 Bypass。
- 工具结果的机器可读副本是 `structuredContent`；Bridge 自己生成的 `content[].text` 是有界摘要，插件原生文本块保持原样，串口工具在其中提供可直接阅读的有界设备回显。`_meta["coterm/operation"]` 只含操作收据，`output` 已替换为大小标记，避免在元信息中再次重复负载。
- Agent commit 默认经过人工审批。Bypass 只跳过提交审批，仍需协议认证及秘密输入。
- 命令审批在桌面「Agent approval」面板配置实际命令文本的正则规则，支持正向放行与反向拦截，保存在加密工作区。规则仅影响宿主审批，不改变 effect、插件校验、秘密交互或来源限制；Agent 不能修改规则或自批。列表聚合、无命令文本时的回退与插件接入见 [命令审批规则](command-approval.md)。
- `effect=read` 且不涉及敏感输入的操作不需要审批；它们只观察状态，不能修改远端。

SSH 认证步骤和工具边界见 [SSH](ssh-plugin.md)，串口可用工具见 [串口](serial-plugin.md)。

### 健康与默认实例

`coterm.status({})` 返回 `desktopAlive`、`bridgeEnabled`、`totalInstances`、`activeInstances`、`connectedInstances`、`instances` 和 `truncated`。每个实例仅有 `name/pluginId/status/connected`；`activeInstances` 是本次快照中 ready 的数量，不代表协议已认证。`connected=true/false/null` 分别表示确认已连接、未连接、未知。最多采样 64 个实例，数量统计针对采样项，`totalInstances` 是总数；超限设置 `truncated=true`，禁止默认路由。

连接状态由插件原生 MCP descriptor 的 `_meta["coterm/connectionStatus"]: true` 声明，每插件最多选择一个状态工具；对应操作必须是实例级、read、非敏感且 transient，结果提供 `structuredContent.connected: boolean`。SSH 的 status 和 Serial 的 status 已声明；未声明、声明歧义、不安全或失败返回未知。基座不猜测协议名称、不执行 connect/probe，不申请认证或写操作审批。探测最多 8 并发，每次等待至多 500ms，整体收集至多 1 秒；慢读不能拖住整个健康检查。

该工具需要已建立的 MCP 连接：返回成功证明桌面与 Bridge 当时可达，不能通过关闭的 Bridge 返回 `desktopAlive:false`。桌面关闭 / Bridge 未启用时仍由 Stdio 适配器在 stderr 明确报连接失败，不能把服务离线理解成健康工具返回的未知实例状态。

## 等待、结果与取消

默认最多等待 60 秒，覆盖秘密交互及执行。SSH 和串口实例工具在进入人工审批时立即返回 `structuredContent.status` 为 `pending_approval` 的结果，带 `operationId/proposalId`；调用方提示用户到桌面审批，再用 `coterm.get_operation` 查询原操作，不重提写入。调用完成直接返回插件结果，`_meta["coterm/operation"]` 附操作收据。立即返回或等待到期时返回当前状态，最终结果可通过查询记录的 `output` 获取。

插件可在注册的工具 descriptor（不是调用参数）中声明 `_meta["coterm/returnOnApproval"]: true`。Bridge 使用运行时已有的状态订阅，在终态或 `awaiting_approval` 时结束本次等待；不轮询账本，不在基座硬编码插件 ID，也不改变普通 `Operations.submit` 的等待行为。未声明的工具仍等待审批、交互和执行直到终态或本次期限。此扩展只影响等待，不能授予审批权限或改变操作的 effect，前端和跨进程调用消息无需变更。

通过调用元信息设置等待上限，不把它混入领域参数。`_meta` 位于 JSON-RPC `tools/call` 的 `params` 顶层，与 `name`、`arguments` 同级：

```json
{
  "name": "coterm_ssh_command",
  "arguments": {"name": "user@example.com:22", "commands": ["pwd"]},
  "_meta": {"coterm/waitTimeoutMs": 0}
}
```

`0` 表示立即返回，`1..300000` 指定毫秒上限。等待超时或客户端断开均不代表底层操作取消，也不触发重放。取消仅在插件确认未提交或已停止时成功，否则可能为 unknown。

部分客户端只向模型暴露 `arguments`，无法设置协议级 `_meta`。这类客户端应省略 `_meta`；SSH / 串口仍会在进入审批时及时返回，其余等待使用默认 60 秒上限。收到 `pending_approval`、运行中或等待到期收据后，使用同一工具返回的 `operationId` 调用 `coterm.get_operation` 查询，不能把 `_meta` 塞进领域参数。服务端保持严格的 `additionalProperties:false` 校验，未知字段会返回 `operation_args_invalid` 并指出未知或缺失的字段名，不回显参数值。

## 适配器传输

- 每会话走一条新建的 TCP 连接，连接建立后直接转发 MCP JSON-RPC 帧；桥只接受本机回环连接。
- 帧为单行 JSON，请求与响应各不超过 1 MiB；超限明确失败，不无限排队。
- 每连接最多 16 个在途工具调用，最多 8 个并发连接；超出返回 JSON-RPC 错误而不是排队。
- 支持 `2025-06-18`、`2025-03-26`、`2024-11-05`；`initialize` 后必须发送 `notifications/initialized` 才能调用工具。
- 目录变化通过 `notifications/tools/list_changed` 通知，适配器逐帧透传，不做事件回放。
- stdio 断开即结束对应桥连接；这不代表底层操作取消，也不会重放。

实现与测试位于 [coterm-mcp](../../crates/coterm-mcp)。Resources、Prompts、订阅、进度和 MCP cancellation notification 尚未实现；`coterm.cancel_operation` 是操作工具。仓库内的 `coterm-mcp-stdio` 二进制保留为开发回归入口，与桌面 `mcp stdio` 共用同一实现。
