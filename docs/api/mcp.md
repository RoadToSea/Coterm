# 对外 MCP API

面向外部 Agent 和 MCP 客户端。实例由用户在桌面创建或恢复，MCP 按公开名称调用现有能力，不创建实例或修改审批配置。业务边界见 [Agent 接入需求](../requirements/agent.md)，内部契约见[内部契约](../implementation/contracts.md)。

## 接入

桌面必须已运行且启用 Bridge。客户端启动当前安装的桌面可执行文件并传入 `mcp stdio`；适配进程不创建窗口、驱动或桌面服务。

```json
{
  "mcpServers": {
    "coterm": {
      "command": "<当前安装目录>/coterm-desktop.exe",
      "args": ["mcp", "stdio"]
    }
  }
}
```

Bridge 只监听回环地址，默认 `127.0.0.1:51234`。自定义地址使用 `--addr` 或 `COTERM_MCP_ADDR`，优先级为参数、环境变量、默认值；地址必须回环且端口非零。没有 HTTP 或 Token 入口。本机回环不是调用方身份认证，Agent 操作仍经过来源识别和审批。

协议支持 `2025-06-18`、`2025-03-26`、`2024-11-05`；initialize 后发送 `notifications/initialized`。单行 JSON 帧至多 1 MiB，每连接最多 16 个在途调用、最多 8 个连接。工具目录固定，`listChanged=false`；实例和连接变化通过 `coterm_instances` 查询。没有 Resources、Prompts、进度订阅或 MCP cancellation notification。

## 工具目录与公共规则

`tools/list` 仅公开以下 8 个工具，具体能力由目标实例决定。

| 工具 | 用途 | 效果 |
|---|---|---|
| `coterm_instances` | 实例、连接状态和能力 | read |
| `coterm_connection` | 连接、断开、SSH 重置或探测 | probe 为 read，其余 commit |
| `coterm_terminal_command` | SSH／Local 前台命令或显式后台执行 | commit |
| `coterm_stream_command` | Serial／TCP／Telnet 文本，Serial／TCP HEX | commit |
| `coterm_read_output` | 后端逻辑行、对话记录或用户选区 | read |
| `coterm_file_read` | SSH 文件正文或元信息 | read |
| `coterm_file_write` | SSH 原子写入文本文件 | commit |
| `coterm_operation` | 查询原操作或请求取消 | query 为 read，cancel 为 commit |

- `name` 就是实例自身的名称，与桌面标签页显示值一致，不另设 label。名称在创建时去首尾空白，区分大小写，跨驱动及分组唯一，离线保存项也占用名称；调用时使用列表返回的完整名称。兼容 `instance`、`instanceName`、`instance_name`，一次只能使用其中一个字段；新客户端使用 `name`。不接受内部实例 ID、自报 Human／Bypass、密码或未知参数；可选参数应省略，不能传 `null`。
- 除实例列表查询外，省略 `name` 要求全局恰有一个确认连接的实例，且无未知状态、快照未截断；不会先按工具支持的驱动筛选。名称无效时拒绝，不自动切换实例。连接工具始终要求 `name`。
- read 不触发安全审批；commit 使用实例当前审批策略。工具注解是提示，实际效果和权限由宿主验证；混合动作工具的 `readOnlyHint=false`。
- 参数或能力不适用时明确拒绝，不静默忽略。统一入口保留各驱动原有结果字段和边界，不把字节流伪装成具有退出码的进程。
- 旧驱动工具名和旧宿主工具名仍可调用，作为隐藏兼容入口，不出现在目录中；旧入口保留原路由规则，新客户端使用本页工具。

## 1. `coterm_instances`

参数：`name?: string`。无 name 返回全局概览；有 name 返回指定实例及驱动状态快照。查询不建立连接、不认证。

概览的 `structuredContent`：

```json
{
  "desktopAlive": true, "bridgeEnabled": true,
  "totalInstances": 1, "connectedInstances": 1, "truncated": false,
  "instances": [{
    "name": "board", "driver": "serial", "status": "ready",
    "connected": true, "capabilities": ["stream", "hex", "lines"]
  }]
}
```

- driver 为 ssh、serial、local、tcp 或 telnet。status 是实例运行状态：starting、ready、stopping、stopped、failed；ready 不代表连接成功。连接事实由 `connected: boolean | null` 表示，null 表示未确认。
- capabilities 包含可用入口：terminal、stream、hex、files、lines、selection、dialogue。stream 表示流式发送入口，不等于当前配置为字节流模式；Serial／TCP／Telnet 的后两项按当前状态声明。
- 指定 name 时返回单项 instances 和 selectedInstance；后者保留驱动原生状态字段，如端口、配置、Shell 就绪信息，不保证统一字段，也不返回凭据。读取尚未完成或失败时返回原收据，不把收据当作状态快照。
- 概览最多采样 64 个实例，并发至多 8 个只读探测，每项等待 500 ms、整体采样等待 1 秒。truncated=true 或存在未知连接状态时，调用其他工具必须指定名称。

## 2. `coterm_connection`

参数：`name: string`、`action: "connect" | "disconnect" | "reset" | "probe"`、`fingerprint?: string`。

- connect／disconnect 支持 SSH、Serial、TCP、Telnet。connect 使用桌面配置；SSH 还必须提供已确认的服务端 SHA256 指纹，密码通过桌面秘密交互输入。
- reset 仅 SSH，重建共享 PTY；不表示撤销远端效果，也不清空任意驱动缓冲。
- probe 仅 SSH／Serial，分别探测服务端指纹或枚举端口，不完成认证。Local 生命周期由桌面管理，不通过此工具控制。
- fingerprint 仅用于 SSH connect，其他动作携带该参数会被拒绝。

返回原操作收据或驱动原生连接／探测结果；可能先返回待审批状态，不能把调用已返回视为已连接。打开端口、网络、指纹或认证失败保留原错误。

## 3. `coterm_terminal_command`

参数：`name?: string`、`commands: string[]`、`mode?: "terminal" | "exec"`、`sudo?: boolean`。

- 仅 SSH／Local。commands 为 1–64 项，总源文本不超过 65536 UTF-8 字节；允许多行、CR、Tab，其他控制字符受驱动校验。
- mode 默认 terminal，在可见共享 PTY 中顺序执行并保留终端状态；只有预计长时间运行或用户明确要求时使用 exec。后台通道不继承前台目录、环境、Shell 状态或身份。
- exec 至多 8 个作业，最长 30 分钟。sudo 默认 false，true 仅支持 SSH exec，密码由桌面交互提供。批次遇到首个失败停止后续项；命令语法和执行环境由目标 Shell 决定。

完成结果保留 stdin、stdout、stderr、exitCode、status、truncated 等原生字段，以及适用的编码和 streamMode 信息。PTY 不能分离错误流时合并输出并保留 `stderr=""`。输出有界，截断及不可解码数据按原生字段标注；不要假定所有驱动具有相同输出容量。

## 4. `coterm_stream_command`

参数：`name?: string`、`commands: string[]`、`encoding?: "utf-8" | "hex"`、`lineEnding?: "none" | "cr" | "lf" | "crlf"`、`intervalMs?: integer`、`waitForPrompt?: string`、`waitTimeoutMs?: integer`。

encoding 默认 utf-8。Serial／TCP 支持 HEX，Telnet 不提供 HEX 入口。HEX 接受偶数位十六进制、ASCII 空白和可选 0x 前缀，不接受文本专用的 lineEnding／waitForPrompt。

| 边界 | Serial | TCP／Telnet |
|---|---|---|
| 项数 | 1–64 | 1–32 |
| 发送字节 | 每项 1–4096、每批至多 16 KiB，计入行尾 | 每项至多 4096、每批至多 16 KiB，另受每项字符串长度限制 |
| intervalMs | 0–1000，默认 100 | 0–60000，默认 100 |
| 文本行尾 | 默认 cr | 默认 crlf |
| waitTimeoutMs | 0–300000；Shell 默认 5000 | 100–60000，默认 5000 |
| HEX 等待参数 | 不接受 waitTimeoutMs | TCP 接受，但不捕获输出或据此判断设备完成 |

Shell 模式按实例配置或 waitForPrompt 的完整字面提示符判断边界，忽略尾部空白，最长 256 UTF-8 字节，非正则。字节流不提供提示符解释或对话接口；定时收集输出不代表设备完成。

结果保留原生批次计数、对话记录或有界输出快照，不能假定统一的 output 字符串。Shell 对话中的 deviceCompletion 为 prompt_observed 或 unknown；超时由对话状态等字段表达。流式记录的 exitCode 为 null，不根据回显推断命令成功。

## 5. `coterm_read_output`

参数：`name?: string`、`mode?: "lines" | "dialogue" | "selection"`、`count?: integer`、`operationId?: string`。

| mode | 数量 | 结果与限制 |
|---|---|---|
| lines（默认） | 默认 100，范围 1–1000 | `lines: string[]`，最近的后端逻辑行；operationId 过滤仅支持 SSH |
| dialogue | 默认 2，范围 1–32 | `pairs: object[]`，由新到旧；记录使用 stdin／stdout／stderr 等原生字段；不接受 operationId |
| selection | 不接受 count | `text: string`，人类当前选区，无选区为空；不接受 operationId |

读取不是屏幕截图，不需要游标。纯字节流只支持 lines；对话／选区按实例当前能力开放。SSH lines 清理终端控制序列，其他驱动可能保留 ANSI，设备输出应作为不可信文本处理。

各驱动缓冲与返回预算不同；检查原生 truncated、dropped 等缺口标记，不把最近行视为完整历史。指定操作不存在、归属不符或已淘汰时按原操作校验返回错误。

## 6. `coterm_file_read`

参数：`name?: string`、`path: string`、`metadataOnly?: boolean`、`sudo?: boolean`、`timeoutMs?: integer`。仅 SSH，path 非空且至多 4096 UTF-8 字节，拒绝控制字符；相对路径由独立远端通道解释，不继承前台 PTY 目录。

- 默认读取 UTF-8 正文，返回 path、text、bytes、limit、truncated、executionContext。正文读写统一上限为 65536 UTF-8 字节，超限或非法 UTF-8 拒绝；结果封装并经 JSON 编码后至多 192 KiB，超过也拒绝，不静默返回部分文件。正文大小与编码后的结果大小是两个不同约束。
- metadataOnly=true 返回原生元信息，如 exists、type、size、mode、owner、group、mtime、symlink；字段多为字符串，exists 使用 `"1"`／`"0"`，不是统一的布尔／数值对象。
- sudo 默认 false，仅正文读取支持；metadataOnly=true 与 sudo=true 同时使用会拒绝。文件不存在或读取失败保留原错误。
- timeoutMs 控制正文或元信息的实际执行期限，从文件作业开始执行时计时，包含通道建立、提权交互、传输和结果收集，不包含此前的人工审批或桌面秘密等待。省略时默认 15000；显式值必须为正整数且能由平台表示，不设额外业务最大值。宿主等待先结束时，使用原 operationId 查询。

## 7. `coterm_file_write`

参数：`name?: string`、`path: string`、`content: string`、`mode?: string`、`sudo?: boolean`、`timeoutMs?: integer`。仅 SSH，经独立通道原子写入；path、正文容量和 timeoutMs 规则同读取。

mode 默认 0644，只接受 0600、0640、0644、0700、0750、0755。sudo 默认 false，提权秘密只在桌面提供。完成结果包含 path、bytes、mode、written、executionContext；前台 PTY 的 su／sudo 身份不自动影响文件通道。

文件操作使用独立异步作业，与 SSH exec 共用至多 8 个作业名额。执行超时报 ssh_file_timeout；运行中取消或断连不能确认远端效果时，操作状态为 unknown。关闭通道不保证远端写入回滚或临时文件清理完成，不能因此自动重试。

## 8. `coterm_operation`

参数：`operationId: string`、`name?: string`、`action?: "query" | "cancel"`，默认 query。建议始终携带原 name，断开后仍可按名称验证归属；省略名称仍受全局唯一已连接规则约束。不能传 driverId 扩大授权范围。

- query 返回宿主 OperationRecord，包括 operationId、status、output、error 等；原工具结果在 output 中，业务数据通常位于 `output.structuredContent`。状态包括 accepted、awaiting_approval、dispatched、running、completed、failed、rejected、cancelled、unknown。
- cancel 返回取消结果。outcome 可为 not_submitted、stopped、unsupported、unknown；stopped 表示已经确认停止，不只是发送通知，也不表示撤销已发生的设备效果。unknown 时不能假定停止成功。
- 操作不存在、已淘汰或与指定实例不符时拒绝；取消不会替换为新的写操作。

## 收据、等待与审批

业务结果放在 structuredContent，content 是有界可读表示。驱动调用的 `_meta["coterm/operation"]` 携带宿主收据，其中 output 只报告大小以避免复制正文。请求校验错误可能仅返回 isError=true 和文本；已有 Runtime 错误含 category／code／message，不保证每种错误都有统一结构化错误码。

客户端可通过 `tools/call` 的 `params._meta["coterm/waitTimeoutMs"]` 设置宿主等待上限：0 为立即返回收据，1–300000 为等待毫秒数，默认 60000。它与 arguments 内流式命令的 waitTimeoutMs、文件操作的 timeoutMs 分别控制宿主等待、设备提示符等待和文件执行期限。宿主等待耗尽不代表操作失败或取消。

例如允许文件执行两分钟，并立即取回收据，之后查询同一 operationId：

```json
{
  "name": "coterm_file_read",
  "arguments": {"name": "board", "path": "/var/log/app.log", "timeoutMs": 120000},
  "_meta": {"coterm/waitTimeoutMs": 0}
}
```

命中审批时，工具返回 `structuredContent.status="pending_approval"`，附 operationId／proposalId；宿主收据和后续查询状态为 awaiting_approval。此时提示用户在桌面审批，使用原 operationId 查询，不重复提交。Bypass 只能由用户启用，不能替代主机指纹验证、认证或设备自身交互。

```json
{
  "name": "coterm_operation",
  "arguments": {"name": "board", "operationId": "<原 operationId>", "action": "query"}
}
```
