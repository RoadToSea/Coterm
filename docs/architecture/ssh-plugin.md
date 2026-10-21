# SSH 插件 MVP

实现位于 `plugins/plugin-ssh`。SSH 连接、主机指纹、认证、PTY、终端输出及界面均归插件；基座提供加载、实例路由、公共秘密输入及操作审批。

## 使用

仓库根目录执行：

```powershell
pnpm install
pnpm plugin:ssh:build
pnpm tauri:dev
```

开发模式自动发现 `plugins/*/dist` 中的完整插件包。发行模式只扫描应用数据目录下的 `plugins`；安装包不包含本机绝对路径。打包脚本仅生成当前平台的后端，manifest 中其他平台的入口需在对应平台构建。

点击左侧 SSH，填写 Host、Port、Username，提交后基座启动插件并创建实例。在实例终端点击 Connect，核对 SHA256 主机指纹后输入密码。同一个插件可创建多个实例；切换实例只卸载界面，不断开 SSH。

当前支持密码认证和交互式 PTY，包括 Ctrl+C、终端尺寸变化。未实现私钥、ssh-agent、keyboard-interactive/MFA、SFTP、端口转发、凭据持久化和自动重连。主机指纹每次连接由用户确认，不自动信任或保存。

## 插件操作

| 操作 | effect | 语义 |
|---|---|---|
| `ssh.status` | read | 配置及连接状态，不含秘密 |
| `ssh.probe` | read | 握手获取 SHA256 指纹，不认证、不打开 Shell |
| `ssh.connect` | commit | 校验确认后的指纹，消费一次性密码引用，创建 PTY |
| `ssh.disconnect` | commit | 释放连接、凭据和该连接的插件提案 |
| `ssh.read` | read | 按十进制字符串游标读取 base64 输出，报告 gap、closed、more |
| `ssh.input` | commit | 人工终端原始字节输入；历史隐藏原始参数 |
| `ssh.command` | commit | 在同一 POSIX PTY 中运行命令并识别完成边界，返回输出与退出码；支持 PrepareProposal/Approve/Reject |
| `ssh.resize` | draft | 只改变终端尺寸 |
| `ssh.plugin_info` | read | 插件级只读操作，不绑定某个 SSH 实例 |

`ssh.input`、`ssh.connect` 是界面的人工作业入口。Agent 使用插件显式注册的 MCP 工具，映射到 `mcp.ssh.*` 内部操作；`mcp.ssh.connect` 不接收密码，审批后请求基座公共密码输入。Agent 命令走 `mcp.ssh.command`，由基座识别来源并执行审批/Bypass；插件准备提案时不发送，批准时使用用户修改后的参数快照。`ssh.input` 的 `submitted=true` 只表示字节交给 SSH 通道；`ssh.command` 则使用下述命令边界等待真实结束。命令块界面仍是后续工作。

**MCP 固定 HTTP 外部连接已接通。** 顶部 MCP 按钮提供固定 URL，无需 Token；插件原生实例级工具、实例发现、审批和秘密输入共用现有实例。插件级工具可不绑定实例，由宿主按插件授权域路由。连接方式见 [固定 MCP 连接](mcp-http.md)，工具及秘密交互见 [MCP Tools 与后端秘密输入](mcp-stdio.md)。Resources、Prompts、订阅和完整双向 provider 尚未实现。

## 本轮公共接口扩展

- `PluginFrontendModule.configure(container, api)`：插件可提供自己的配置表单；`api.submit(config)` 由基座启动插件、创建实例。没有此导出的插件仍可使用 JSON 配置。
- `plugin_frontend_source(pluginId)`：只返回已验证插件包的前端入口，最多 2 MiB。MVP 使用无相对依赖的单文件 ES bundle，经 Blob URL 加载；样式由插件封装。本插件使用 Shadow DOM 隔离样式。同 WebView 可信模块不是安全沙箱。
- `requestSecret(...)`：基座弹窗收集密码，插件前端只收到 `SecretHandle`。基座通过独立 `provide_secret` 方法送入该实例后端，跳过普通操作、账本和 MCP；引用绑定实例、用途及 30 秒有效期，使用一次即销毁。插件将待用秘密保存在 `Zeroizing<String>`；不承诺所有底层库与 IPC 临时副本都已内存擦除。
- `OperationDescriptor.sensitiveInput`：可选，默认为 false；原始终端按键可能含密码，此类输入及批准快照在普通账本中替换为脱敏占位。
- `create_instance` 响应可包含 `label`：宿主只验证长度及控制字符，SSH 插件返回 `username@host:port`，宿主不解析协议配置。

后端可通过延迟交互控制应答请求密码，释放 RPC 后等待基座发送 `interaction_result` 恢复同一操作。插件不需要前端页面也能使用公共密码弹窗；尚无永久凭据、MFA 或通用秘密兑换 SDK。私有秘密方法仍与控制消息共用物理 stdio，后续可拆分。

## 数据限制与验证

每连接最多缓存 256 KiB 原始输出，每块最多 8 KiB，单次读取不超过 8 KiB，使用 base64 避免损坏非 UTF-8 字节。界面先安装事件订阅，再读取一次状态和回放快照，随后只消费推送；连接变化或报告缺口时按游标回放，空闲时不读取输出或状态。慢消费者读取已过期游标时收到 gap。前端输入队列不超过 16 KiB，后端控制队列最多 64 项，终端滚动缓存 3000 行。JSON 事件适配器和消费确认已接通；正式二进制流及 StreamRouter 接口尚未接通。

普通账本保留最近 N 条；`ssh.read`、`ssh.status`、`ssh.resize`、人工 `ssh.input` 标记为 transient，使用另一份有界容量，不挤占命令历史。需要审批的操作仍进入普通账本。非终态保留；全部容量被活动记录占满时拒绝新操作。

测试使用本机随机端口的 russh 测试服务器，覆盖错误指纹、错误密码、双实例隔离、PTY 收发、输出缺口、密码引用隔离及一次性消费，以及基座真实子进程审批快照和敏感输入脱敏；外部 MCP 测试还覆盖插件原生工具、后端密码交互、Bypass、迟到输入拒绝、共享 PTY、跨实例查询拒绝及目录变化通知。另外覆盖原 MCP 请求跨审批等待、同连接并发状态查询、完成输出及非零退出码、立即返回、断线后 unknown、后台读取与命令账本隔离。边界解析测试覆盖逐字节分片和有界捕获；可用 `sh` 时验证真实 POSIX Shell 的引号、退出码和环境状态延续。更新后的链路仍需远程实机验收，未覆盖所有操作系统。

## 命令边界与结果

SSH 插件声明 `async-operations` 和 `plugin-events`。命令提交后控制 RPC 返回 `{ "_cotermDeferred": "operation-id" }`，释放消息循环；插件随后通过 `coterm.notification` 报告同一操作的 `operation_completed` 或 `operation_failed`。基座不解析 Shell，也不为其他协议推断命令结束。

`ssh.command` 要求远端为 POSIX 兼容 Shell，并处于可接收命令的位置。插件将已批准的单行命令以单引号转义后交给当前 Shell 的 `eval`，在前后打印随机令牌标记，并记录 `$?`。因此 `cd`、`export` 等仍作用于共享 Shell；终端字节经分片解析后移除实际标记，命令回显可能包含包装脚本。当前不支持自动检测已有未完成的人工命令、提示符集成或远端 Windows Shell。

结果包含 base64 输出、退出码及截断标志，最多捕获 8 KiB；共享终端仍保留自身 256 KiB 回放。源命令最多 4096 字节，转义并包装后还须满足 4000 字节的 PTY 单行上限。每连接同一时刻只接受一个受跟踪命令；人工原始输入仍可用于交互和 Ctrl+C。

命令最长跟踪 120 秒，与调用方的等待超时分别计时。`exit`、`exec`、不兼容 Shell、人工中断等可能让结束标记缺失；取消操作会尽力发送 Ctrl+C，但不能确认远端进程是否停止，因此宿主记录 `unknown`。断线或跟踪到期也报告 `unknown`，不从提示符或输出静默推断成功，也不重发命令。到期后仍拒绝下一条受跟踪命令，直到收到原结束标记或重新连接，避免把运行中终端误当成空闲。后台任务结束不属于前台命令的完成语义。

## 界面输出通道

插件发送通用实例事件 `ssh.output`（连接 ID、十进制游标、base64 字节）和 `ssh.status`。基座只按插件实例转发。SSH 插件使用 128 项有界通知队列；宿主为每个订阅保留最多 64 项广播事件，慢消费者收到 `runtime.gap` 后向插件回放，不影响其他订阅。连接 ID 防止旧连接事件被应用到新连接。

桌面通过 `PluginAPI.subscribeEvents` 的 Tauri Channel 传递事件，每次最多一条常规事件等待前端完成处理并确认，最多 32 个订阅，30 秒未确认则发送关闭原因并释放。取消订阅或卸载界面不关闭 SSH。前端每次恢复最多读取 64 批，所有缓存均有界；持续超量的数据可能产生明确缺口。本适配器使用 JSON/base64，不代表正式二进制流接口已冻结。
