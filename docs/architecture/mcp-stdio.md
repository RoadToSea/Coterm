# MCP Tools、秘密输入与旧 stdio 适配器

本页描述当前可运行的实现；[MCP 集成](mcp-integration.md)中的 Resources、Prompts、订阅和完整双向协议仍是后续设计。

## 启动和连接

桌面当前使用固定的 Streamable HTTP 地址 `http://127.0.0.1:23517/mcp`。用户只填写 URL，无需 Token，应用重启后不变；配置方法、会话与访问边界见 [固定 MCP 连接](mcp-http.md)。

旧 `coterm-mcp-stdio` 转接程序和 `BridgeServer` 仍保留用于开发回归测试，通过 `COTERM_MCP_ADDR` 和 `COTERM_MCP_TOKEN` 连接测试中启动的私有回环服务。桌面不再启动该随机端口服务，也不再导出包含 Token 的配置；旧客户端配置应改为上述 HTTP URL。该转接器不具备自动发现新 HTTP 服务的能力。

下面的 Tools、操作等待、审批与秘密交互规则由两种传输共用；stdio 的物理帧限制仅描述旧适配器。

## 可用工具

基座提供：

| 工具 | 用途 |
|---|---|
| `coterm.list_instances` | 查找用户已创建的实例及 `pluginInstanceId` |
| `coterm.get_operation` | 按实例和操作 ID 查询审批、交互及最终结果 |
| `coterm.cancel_operation` | 请求取消操作；返回状态取决于插件能否确认未提交或已停止 |

SSH 插件显式注册 `ssh.status`、`ssh.probe`、`ssh.connect`、`ssh.disconnect`、`ssh.command` 和 `ssh.read`。对外工具名为 `coterm-plugin-ssh__ssh.command` 等；参数保留插件定义，必须明确传入 `pluginInstanceId`。原始按键与密码输入不暴露为 MCP 工具。

插件在运行时握手的 `mcpTools` 中提交原生 MCP Tool descriptor 和对应的内部 `operation`。基座只修改工具名，按内部操作目录确定 scope/effect 和审批策略；**不会把整个操作目录自动变为 MCP 工具**。`mcp.*` 是 SSH 插件选择的内部操作名，不是所有插件必须采用的命名格式。

支持实例级和插件级 Tools。实例级工具的参数 schema 必须包含必填字符串 `pluginInstanceId`；插件级工具不得依赖当前标签页或隐式实例，宿主按已注册工具所属插件建立授权域。公共 `coterm.get_operation` / `coterm.cancel_operation` 可用 `pluginInstanceId` 或 `pluginId` 指定记录授权域。插件调用完成时必须返回自己构造的 MCP `CallToolResult`，例如 `content`、`isError`；基座保留原生结果，并在 `_meta["coterm/operation"]` 中附加操作凭据。默认等待原操作终态后直接返回；仅立即返回或等待超时时返回公共 pending 结果，调用方再通过 `coterm.get_operation` 读取状态，最终原生结果位于记录的 `output`。插件如声明 `outputSchema`，须兼容 pending 结果；SSH MVP 暂不声明此 schema。

所有外部 MCP 调用来源固定为 Agent。Agent 不能通过参数改变来源、effect、Bypass 或批准自己的操作。当前本机客户端共用用户授权的实例权限，不提供多用户或每客户端 ACL。

## SSH 连接流程

1. `coterm.list_instances` 获取已有实例；实例配置和创建当前由桌面完成。
2. `ssh.probe` 获取服务器指纹。应通过可信来源核对指纹，探测本身不代表信任服务器。
3. 调用 `ssh.connect`，传入实例 ID 和核对后的 `fingerprint`，不传密码。
4. 未开启 Bypass 时，基座显示审批草稿。用户可修改参数后批准或拒绝。
5. 批准后插件请求密码；基座显示独立的密码弹窗。不开启插件页面、插件不提供 UI，也可以使用这个公共入口。
6. 密码通过私有 `provide_secret` 交给指定插件实例；普通 `interaction_result` 只携带一次性、限用途、30 秒有效的句柄。插件消费句柄后恢复连接操作。
7. 原 MCP 请求默认等待审批、密码输入和连接结果，连接成功后直接返回。`ssh.command` 和人工终端共用同一 PTY；SSH 插件识别 POSIX Shell 命令边界并返回有界输出及退出码，页面通过基座接收输出事件。`ssh.read` 供按需回放和缺口恢复。

Bypass 只跳过基座的提交审批，不替代主机校验、密码认证或其他协议交互。用户取消、60 秒交互超时、实例销毁或插件停止后，旧交互不能再次恢复操作；断开/重新连接也会在 SSH 后端使旧交互失效。秘密不进入普通操作记录和 MCP 返回值；终端远端自身回显的内容仍由远端控制。

## 等待与立即返回

基座受理时分配操作 ID，再异步派发。默认等待 60000ms，包含人工审批、秘密交互和插件执行；等待不会占用插件控制请求或阻塞后续 MCP 调用。每个客户端最多 16 个在途调用，结果按 JSON-RPC ID 关联，不保证请求顺序。

Tauri/PluginAPI 使用调用选项 `waitTimeoutMs`。MCP 使用 `tools/call.params._meta["coterm/waitTimeoutMs"]`，与插件领域参数分开：

```json
{
  "name": "coterm-plugin-ssh__ssh.command",
  "arguments": {"pluginInstanceId": "instance-id", "text": "pwd"},
  "_meta": {"coterm/waitTimeoutMs": 0}
}
```

`0` 返回 `accepted` 收据；`1..300000` 指定等待上限。不提供时等待 60 秒。到期返回当前收据及 `waitExpired=true`，不撤销审批或执行，也不自动重放。只有拿到非终态收据时，才需要按操作 ID 查询。客户端自身的超时也不能被解释为命令没有发送。

SSH `ssh.command` 正常结果的 `content[0].text` 是 JSON，包含 `data`（base64）、`encoding`、`exitCode`、`outputTruncated`。最多捕获 8 KiB 命令输出；非零退出码保留实际输出，原生 MCP `isError=true`。连接中断或无法确认命令结束时，操作返回 `unknown`，不能按成功处理。限制见 [SSH 命令边界](ssh-plugin.md#命令边界与结果)。

账本将 `OperationDescriptor.transient=true` 的后台读取和交互输入放入独立的有界区，每实例两个区各最多 100 条；`recentOperations` 只列普通操作，单条 transient 结果仍可用操作 ID 查询。需要审批的操作始终进入普通区。普通操作按最近 N 步保留，仍不永久存储；等待中的调用持有结果通知，即使终态记录随后被其他普通操作淘汰，也能收到最终结果。

## 插件作者的延迟交互契约

插件声明 `native-mcp-tools`、`deferred-interactions`，并声明 `secret-input` capability。当前 MVP 不要求插件在等待用户时阻塞消息循环或主动向 stdout 写通知。`invoke` / `approve` 的 JSON-RPC 应答可以使用保留的控制信封：

```json
{
  "_cotermInteraction": {
    "interactionId": "password-for-op-17",
    "pluginInstanceId": "instance-1",
    "operationId": "op-17",
    "kind": { "kind": "secret", "purpose": "ssh.password" },
    "title": "SSH authentication",
    "prompt": "Password for this instance",
    "timeoutMs": 60000,
    "allowCancel": true
  }
}
```

插件自行保存有界的待恢复上下文，立即结束本次 RPC。基座消费控制信封，将原操作记为 `running`，不将它当作领域结果。交互完成后，基座发送 `interaction_result`；插件校验交互 ID、实例、用途、有效期及一次性句柄，再返回原操作的最终结果。当前支持一次秘密交互，尚不实现多个连续交互、MFA 或通用 SDK 兑换。

交互最多 32 个，宿主将期限限制在 1～300000ms，桌面每秒处理到期任务。待审批和其他非终态操作不会被轮询记录淘汰；每个实例账本容量全部被活动操作占满时，拒绝新操作。宿主销毁实例或停止插件时清理其交互；插件也必须清理自己的待恢复上下文。

当前秘密方法仍和普通控制消息共用物理 stdio，仅做逻辑隔离；同 WebView 可信前端模块也不是安全沙箱。后续可拆分秘密物理通道并增加独立前端隔离。

## 传输范围和限制

旧适配器使用换行分隔的 JSON-RPC stdio；当前桌面 HTTP 传输见 [固定 MCP 连接](mcp-http.md)。二者支持初始化协商、ping、Tools 列表、调用和目录变化通知；启动/停止插件后，已连接客户端会收到 `notifications/tools/list_changed`。不声明尚未实现的 Resources、Prompts、进度或订阅能力。

旧 BridgeServer 只监听 IPv4 回环随机端口，认证限时 5 秒，最多 8 个客户端；每帧最多 1 MiB，认证和协议数据不会混入转接程序 stdout。桌面退出时停止监听并关闭所属客户端任务。每客户端可并发等待；插件单次控制应答仍串行且限时 10 秒，长任务必须立即应答延迟受理，再用完成通知报告结果。等待审批、密码或 Shell 命令完成时不占用控制 RPC；实际连接握手最长 7 秒，期间其他控制请求仍会排队。外部客户端断开不会自动撤销已提交或待审批操作，用户仍可在桌面拒绝；不自动重试写操作。

已支持默认等待、指定等待超时和 `waitTimeoutMs=0`。取消支持尚未派发、待审批、待密码交互操作，也会向运行中的插件发送取消请求。只有插件确认未提交或已停止时结果才为 `cancelled`；SSH 发送 Ctrl+C 后无法确认远端状态，因此返回 `unknown`。MCP cancellation notification、正式双向 MCP provider 和二进制流尚未实现。

测试覆盖真实 stdio 转接子进程、认证失败、帧边界、真实 SSH 插件子进程与本地测试服务器之间的 MCP 路由、审批快照、密码隔离、共享终端、实例定位、Bypass、取消及目录变化。2026-10-21 的历史记录报告：人工修改审批后，原 MCP 请求直接返回修改后的输出与退出码 0；另一条使用默认等待的命令准确返回退出码 1 和 `isError=true`，无需轮询操作结果。高频读取的记录隔离已有自动化验证。上述远程实机结论未在本次合并中重测；当前可复跑验证和仍待验收的范围见[项目进度评估](../PROGRESS.md)。
