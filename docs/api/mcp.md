# 对外 MCP API

本文件面向外部 Agent 和 MCP 客户端，说明接入方式、对外工具、参数、返回值与调用限制。实例应先由用户在桌面创建或恢复。内部控制、驱动配置和容量实现见[内部契约](../implementation/contracts.md)，业务要求见[Agent 接入需求](../requirements/agent.md)。

## MCP 接入

外部客户端启动桌面可执行文件，参数为 `mcp stdio`。桌面必须已运行且启用 Bridge；该适配模式不创建窗口或驱动、不启动桌面服务。客户端 command 使用当前安装可执行文件路径，不依赖 PATH。

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

Bridge 只监听 `127.0.0.1`，默认端口 `51234`。自定义端口的复制配置包含 `--addr`，例如 `127.0.0.1:12345`；也接受 `COTERM_MCP_ADDR`，优先级为参数、环境变量、默认值。地址必须回环且端口非零；无 Token 或 HTTP 入口。本机回环不是调用方身份认证，Agent 操作仍经过来源识别和安全审批。

协议支持 `2025-06-18`、`2025-03-26`、`2024-11-05`；initialize 后需发送 `notifications/initialized`。单行 JSON 帧各至多 1 MiB，每连接最多 16 个在途调用、最多 8 个连接。支持 `tools/list_changed`；尚无 Resources、Prompts、进度、订阅或 MCP cancellation notification。

## 工具总览与路由规则

对外工具统一采用 `coterm_<功能>` 命名，按操作能力与交互语义划分，不再按底层驱动机械分散。工具分为无需审批的只读类（`read`）与受控进入审批的变更类（`commit`）。

| 工具名称 | 效果 | 核心用途 |
|---|---|---|
| `coterm_instances` | read | 查询实例列表、连接快照与驱动能力 |
| `coterm_connection` | commit | 控制实例的连接建立、主动断开与会话重置 |
| `coterm_terminal_command` | commit | 面向 POSIX/Shell 终端（SSH、Local）执行批量命令 |
| `coterm_stream_command` | commit | 面向字符流与总线设备（Serial、Raw TCP、Telnet）发送指令或 HEX 报文 |
| `coterm_read_output` | read | 统一读取终端历史逻辑行、交互对话对或用户选区 |
| `coterm_file_read` | read | 读取远程主机文件内容或元信息（SSH） |
| `coterm_file_write` | commit | 受控原子写入远程主机文件（SSH） |
| `coterm_operation` | read / commit | 查询长任务与在途审批状态，或请求取消操作 |

### 实例目标选择规则

- 实例参数 `name` 为目标实例的公开名称（不公开内部 UUID）。兼容 `instance`、`instanceName`、`instance_name` 别名传入，一次只能使用一个名称字段。
- **目标省略条件**：仅在全局恰有一个确认已连接实例、无未知状态且快照未截断时才能省略 `name`；存在多连接、无连接、未知状态或名称显式无效时直接拒绝，不自动回退或隐式切换。
- **操作归属**：查询或取消在途操作时，建议显式提供原 `name`，服务端验证操作与目标归属的一致性。

---

## 接口详细规范

### 1. `coterm_instances`

查询桌面当前所有实例的连接状态与驱动能力，或检索单个实例的详细配置快照。

- **效果**：`read`
- **参数说明**：
  - `name`（string，可选）：目标实例名称。省略时返回全局实例概览；提供时返回指定实例的详细快照。
- **返回值**：
  - `instances`（array）：实例列表。每个对象包含：
    - `name`（string）：公开实例名称。
    - `driver`（string）：驱动类型，取值为 `ssh`、`serial`、`local`、`tcp`、`telnet`。
    - `status`（string）：连接状态，取值为 `connected`、`disconnected`、`connecting`、`unknown`。
    - `capabilities`（array<string>）：实例支持的能力清单，如 `["terminal", "files"]` 或 `["stream", "hex"]`。
    - `activeOperations`（number）：当前实例正在排队或执行的在途操作数。
  - `selectedInstance`（object，仅当传入 `name` 时返回）：包含波特率、主机端口、终端模式等当前配置快照。
- **边界与异常**：
  - 传入的 `name` 不存在时返回 `not_found` 错误。
  - 状态采样最多 64 个实例，单次探测并发至多 8、超时 500 ms，查询不会触发实际建立连接或认证。

### 2. `coterm_connection`

管理目标实例的底层连接生命周期，支持显式连接、断开与会话重置。

- **效果**：`commit`
- **参数说明**：
  - `name`（string，必填）：目标实例名称。
  - `action`（string，必填）：操作动作，取值限定为：
    - `"connect"`：建立底层连接并使用桌面已保存的凭据完成认证。
    - `"disconnect"`：终止当前连接并释放端口或网络套接字。
    - `"reset"`：在保持连接的前提下重建共享 PTY（仅 SSH/Local 有效）或清空通信缓冲。
    - `"probe"`：未认证状态探测（获取服务端指纹或枚举物理端口，不打开设备）。
  - `fingerprint`（string，可选）：SSH 连接时必填，必须匹配远端主机的 SHA256 指纹；非 SSH 忽略。
- **返回值**：
  - `status`（string）：操作后实例状态（`connected`、`disconnected`、`probing`）。
  - `fingerprint`（string，可选）：探测或连接时远端返回的实际指纹。
  - `ports`（array<string>，可选）：串口探测返回的本地可用端口列表。
- **边界与异常**：
  - SSH 连接未提供或指纹不匹配时明确返回 `conflict` 错误并中止。
  - 目标实例已被占用或端口打开失败返回 `transport` 错误。

### 3. `coterm_terminal_command`

面向 POSIX/Windows Shell 等具备标准操作系统交互与退出码机制的终端实例（SSH、Local）执行批量命令。

- **效果**：`commit`
- **参数说明**：
  - `name`（string，可选）：目标实例名称。
  - `commands`（array<string>，必填）：待执行的命令列表，限制 1–64 项，总字符长度不超过 65536 UTF-8 字节。允许多行、CR、Tab，拒绝其它非法控制字符。
  - `mode`（string，可选）：执行模式，默认为 `"terminal"`：
    - `"terminal"`：在前台共享 PTY 中顺序执行，输出合并入终端主历史。
    - `"exec"`：在后台独立子 Shell 中启动作业，具备独立的错误流与超时隔离（至多 8 个作业，上限 30 分钟）。
  - `sudo`（boolean，可选）：默认 `false`。是否请求提权执行（仅在 `mode="exec"` 时允许为 `true`；密码由桌面安全通道代填，不进入参数）。
- **返回值**：
  - 返回标准宿主收据 `operationId`、`status` 及执行结果记录：
    - `stdin`（string）：执行的源指令。
    - `stdout`（string）：标准输出文本（UTF-8，不可解码时转为 base64 并设置 `stdoutEncoding="base64"`）。
    - `stderr`（string）：标准错误文本（仅在独立通道存在时返回，PTY 合并流中为空）。
    - `exitCode`（number | null）：命令进程退出码。
    - `truncated`（boolean）：输出是否超过单次缓冲区上限（64 KiB）并被截断。
- **边界与异常**：
  - 流式设备（Serial、TCP、Telnet）调用此接口返回 `unsupported` 错误。
  - 列表执行过程中遇到首个非零退出码时立即终止后续命令，并将批次整体标记为退出状态。

### 4. `coterm_stream_command`

面向无进程概念、基于字符流或特定提示符交互的总线与网络控制台（Serial、Raw TCP、Telnet）发送指令或原始报文。

- **效果**：`commit`
- **参数说明**：
  - `name`（string，可选）：目标实例名称。
  - `commands`（array<string>，必填）：待发送报文列表，1–64 项，单批发送总量至多 16 KiB。
  - `encoding`（string，可选）：数据编码格式，默认为 `"utf-8"`：
    - `"utf-8"`：普通文本交互。
    - `"hex"`：十六进制原始报文（支持偶数位十六进制字符、ASCII 空白和可选的 `0x` 前缀）。
  - `lineEnding`（string，可选）：文本模式追加的行尾换行符，取值 `"cr"`、`"crlf"`、`"lf"`、`"none"`。Serial 默认 `"cr"`，TCP 默认 `"crlf"`。HEX 模式忽略此参数。
  - `intervalMs`（number，可选）：批次项之间的发送间隔，范围 0–60000 ms，默认 100 ms。
  - `waitForPrompt`（string，可选）：文本模式下每项发送后等待的目标提示符完整字面行（忽略尾部空白，最长 256 UTF-8 字节，非正则）。
  - `waitTimeoutMs`（number，可选）：单项等待提示符或流输出的有界等待超时，0–300000 ms，默认 5000 ms。
- **返回值**：
  - 标准宿主收据 `operationId` 及流式执行记录：
    - `output`（string）：等待窗口内捕获的设备回显文本。
    - `deviceCompletion`（string）：完成证据，取值 `"prompt_observed"`（命中指定提示符）、`"timed_out"`（超时返回）、`"unknown"`。
    - `exitCode`：恒为 `null`（流式设备无退出码）。
- **边界与异常**：
  - HEX 格式非法（奇数位、非 Hex 字符）立即报错 `invalid_request`。
  - POSIX 实例（SSH/Local）调用此接口返回 `unsupported` 错误。

### 5. `coterm_read_output`

统一读取任意实例的终端屏幕或流式接收历史，无需 Agent 自行维护游标或内部偏移。

- **效果**：`read`
- **参数说明**：
  - `name`（string，可选）：目标实例名称。
  - `mode`（string，可选）：读取模式，默认为 `"lines"`：
    - `"lines"`：读取终端最近滚动的逻辑行文本（已剔除终端控制序列）。
    - `"dialogue"`：读取最近命令与响应构成的对话对（由新到旧排列）。
    - `"selection"`：读取当前桌面人类用户在终端中鼠标高亮选中的文本（无选区时返回空文本，最多 16 KiB）。
  - `count`（number，可选）：读取数量上限。`mode="lines"` 时默认 100（范围 1–1000）；`mode="dialogue"` 时默认 2（范围 1–32）。
  - `operationId`（string，可选）：指定读取某一历史命令独立记录的输出。
- **返回值**：
  - `lines`（array<string>，当 `mode="lines"` 时返回）：按 LF 切分的逻辑行数组。
  - `pairs`（array<object>，当 `mode="dialogue"` 时返回）：对话对数组，每项包含 `command` 与 `response`。
  - `text`（string，当 `mode="selection"` 时返回）：选区纯文本。
  - `truncated`（boolean）：底层环形缓冲是否有更早的历史被淘汰。
- **边界与异常**：
  - 纯 stream 模式实例不支持对话提取，请求 `mode="dialogue"` 返回 `unsupported`。
  - 指定的 `operationId` 不存在或已被账本淘汰时返回 `not_found`。

### 6. `coterm_file_read`

读取远程主机文件内容或元信息（目前支持 SSH 驱动）。经独立安全通道传输，不破坏前台 PTY 状态。

- **效果**：`read`
- **参数说明**：
  - `name`（string，可选）：目标实例名称。
  - `path`（string，必填）：远程文件绝对路径（1–4096 字符）。
  - `metadataOnly`（boolean，可选）：默认 `false`。为 `true` 时仅获取文件状态元数据，不读取正文。
  - `sudo`（boolean，可选）：默认 `false`。是否提权读取。
- **返回值**：
  - `exists`（boolean）：文件是否存在。
  - `size`（number）：文件字节大小。
  - `mode`（string）：权限掩码（如 `"0644"`）。
  - `modified`（string，可选）：最后修改时间戳。
  - `content`（string，可选）：文件 UTF-8 文本内容，单次读取上限 8192 字节。
- **边界与异常**：
  - 文件超过 8192 字节时拒绝返回正文，要求通过 `coterm_terminal_command` 流式导出或分块处理。
  - 路径不存在且 `metadataOnly=false` 时返回 `not_found` 错误。
  - 非 SSH 驱动实例调用返回 `unsupported` 错误。

### 7. `coterm_file_write`

原子写入远程主机文件内容（目前支持 SSH 驱动）。经独立传输通道，单次写入期限 15 秒。

- **效果**：`commit`
- **参数说明**：
  - `name`（string，可选）：目标实例名称。
  - `path`（string，必填）：远程文件绝对路径（1–4096 字符）。
  - `content`（string，必填）：写入文件的完整文本，上限 65536 字节。
  - `mode`（string，可选）：目标权限掩码，默认 `"0644"`。仅允许 `0600`、`0640`、`0644`、`0700`、`0750`、`0755`。
  - `sudo`（boolean，可选）：默认 `false`。是否提权写入。
- **返回值**：
  - `bytesWritten`（number）：实际写入字节数。
  - `path`（string）：写入的目标路径。
- **边界与异常**：
  - 传入非白名单的 `mode` 权限掩码时直接拒绝并返回 `invalid_request`。
  - 传输超时或远端磁盘配额不足返回 `transport` 或 `domain` 错误。

### 8. `coterm_operation`

用于跟踪异步长任务、审批状态或请求撤销在途操作。

- **效果**：`action="query"` 为 `read`；`action="cancel"` 为 `commit`
- **参数说明**：
  - `operationId`（string，必填）：操作唯一标识符。
  - `action`（string，可选）：操作类型，默认为 `"query"`：
    - `"query"`：查询原操作执行收据与机器结果。
    - `"cancel"`：请求取消当前正在排队或执行的操作。
  - `name`（string，可选）：目标实例公开名称，用于归属校验。
- **返回值**：
  - `operationId`（string）：操作标识。
  - `status`（string）：收据状态，取值为 `accepted`、`awaiting_approval`、`dispatched`、`running`、`completed`、`failed`、`rejected`、`cancelled`、`unknown`。
  - `output`（string | object，可选）：已完成或当前截断的输出结果。
  - `outcome`（string，仅 `action="cancel"` 时返回）：撤销结果，取值为 `not_submitted`（在途队列直接移出）、`stopped`（已通知驱动终止）、`unsupported`、`unknown`。
- **边界与异常**：
  - `operationId` 不存在或已被清理时返回 `not_found`。
  - 操作关联实例与提供的 `name` 不匹配时返回 `conflict`。

---

## 等待、安全审批与取消机制

### 协议等待机制

客户端在 `tools/call` 的 `params._meta["coterm/waitTimeoutMs"]` 中指定等待上限：
- `0`：立即返回在途收据（非阻塞模式）。
- `1`–`300000`：指定等待完成的超时毫秒数。
- 省略时默认等待 `60000` ms。
- *注意*：该等待参数控制的是 JSON-RPC 往返等待，不是工具业务参数内的 `waitTimeoutMs`。

### 审批工作流

所有 `commit` 效果的工具调用均受到桌面审批策略约束：
1. 若命中审批规则，服务端返回收据并标记 `structuredContent.status="pending_approval"`，携带 `operationId` 与 `proposalId`。
2. 宿主收据状态更新为 `awaiting_approval`。
3. Agent **严禁**在此状态下重复提交同一指令，应提示用户在桌面审核，并周期性调用 `coterm_operation(action="query", operationId=...)` 查询终态结果。

### 结果封包格式

- `structuredContent`：存放类型化结构体，供 Agent 进行逻辑判断与决策。
- `content`：返回有界可读文本摘要，供控制台展示。
- `_meta["coterm/operation"]`：携带完整宿主收据元数据，但 `output` 会被替换为字节大小标记，防止多重冗余复制。
