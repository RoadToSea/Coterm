# 对外 MCP API

本文件面向外部 Agent 和 MCP 客户端，说明接入方式、工具、参数、返回值与调用限制。实例应先由用户在桌面创建或恢复。内部控制、驱动配置和容量实现见[内部契约](../implementation/contracts.md)，业务要求见[Agent 接入需求](../requirements/agent.md)。

## MCP 接入

外部客户端启动桌面可执行文件，参数为 mcp stdio。桌面必须已运行且启用 Bridge；该适配模式不创建窗口或驱动、不启动桌面服务。客户端 command 使用当前安装可执行文件路径，不依赖 PATH。

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

Bridge 只监听 127.0.0.1，默认端口 51234。自定义端口的复制配置包含 --addr，例如 127.0.0.1:12345；也接受 COTERM_MCP_ADDR，优先级为参数、环境变量、默认值。地址必须回环且端口非零；无 Token 或 HTTP 入口。本机回环不是调用方身份认证，Agent 操作仍经过来源识别和审批。

协议支持 2025-06-18、2025-03-26、2024-11-05；initialize 后需发送 notifications/initialized。单行 JSON 帧各至多 1 MiB，每连接最多 16 个在途调用、最多 8 个连接。支持 tools/list_changed；尚无 Resources、Prompts、进度、订阅或 MCP cancellation notification。

## 公共工具与路由

| 工具 | 参数 | 用途 |
|---|---|---|
| coterm.list_instances | 无 | 实例名称、驱动和状态，不公开内部 UUID |
| coterm.status | 无 | 桌面和 Bridge 可达时的实例连接快照 |
| coterm.get_operation | operationId 必填，name 或 driverId 可选且互斥 | 查询原操作，验证目标归属 |
| coterm.cancel_operation | 同上 | 请求取消，不承诺已经撤回设备操作 |

实例工具公共参数 name 可选，但仅在目标驱动恰有一个确认已连接实例、无未知状态且快照未截断时才能省略。显式无效名称不能回退；兼容 instance／instanceName／instance_name，一次只能使用一个名称字段。driver_info 是驱动级工具，不接受 name。

查询／取消工具同时省略 name 和 driverId 时，要求全局唯一确认连接实例且没有未知状态。查询旧操作推荐始终传原 name。

所有工具拒绝未声明参数。工具名形如 coterm_ssh_command，由固定驱动名称和原生工具名称规范化而来；规范化冲突明确拒绝。旧双下划线形式仅作隐藏兼容别名，不重复列出。

健康快照最多采样 64 个实例，connected 为 true／false／null；activeInstances 统计 ready，不代表认证连接。超限禁止默认路由；状态探测最多 8 并发、每次 500 ms、整体 1 秒，不调用 connect 或认证。

## SSH：14 个工具

下表省略公共 name，问号表示可选。read 不经过写审批，commit 仍受审批、认证和参数校验。

| 工具 | 效果 | 业务参数／边界 |
|---|---|---|
| coterm_ssh_driver_info | read | 无，驱动级 |
| coterm_ssh_status | read | 无 |
| coterm_ssh_probe | read | 无，未认证指纹探测 |
| coterm_ssh_connect | commit | fingerprint 必填，核对 SHA256 指纹；密码走桌面 |
| coterm_ssh_disconnect | commit | 无 |
| coterm_ssh_reset | commit | 无，重建共享 PTY，exec 不受影响 |
| coterm_ssh_command | commit | commands 必填，mode?=terminal／exec，sudo?=false |
| coterm_ssh_read_lines | read | lines?=100，1–1000；operationId? |
| coterm_ssh_read_selected_lines | read | 无；无选区时 text 为空，最多 16 KiB |
| coterm_ssh_read_dialogue_pairs | read | count?=2，1–32，新到旧 |
| coterm_ssh_system_info | read | 无，固定系统信息查询 |
| coterm_ssh_file_info | read | path 必填，元信息 |
| coterm_ssh_read_file | read | path 必填，sudo?；至多 8192 字节 UTF-8 |
| coterm_ssh_write_file | commit | path、content 必填，mode?=0644，sudo?；至多 65536 字节 |

command 接受 1–64 项，总源文本至多 65536 UTF-8 字节；允许多行、CR、Tab，拒绝其他控制字符。列表首个非零退出停止，整个批次形成一条命令记录。terminal 默认共享 PTY；exec 独立 Shell，最多 8 个作业、作业上限 30 分钟。sudo 命令必须 mode=exec；密码不进入参数。

路径 schema 为 1–4096 字符；写入权限只允许 0600／0640／0644／0700／0750／0755。文件经独立通道，期限 15 秒，原子替换；不继承 PTY 的提权状态。提权交互支持一次密码或 NOPASSWD，TTY／MFA 等额外认证明确失败，不自动重试。

read_lines 按 LF 逻辑行，剔除终端控制序列，不模拟屏幕；可按 operationId 读取运行中或结束命令的两流。目标不存在或已淘汰报错，不回退其他记录。仅 Human 的 select_lines 更新选区，绑定 connectionId；块选择不是文本选区。

## Serial：10 个工具

| 工具 | 效果 | 业务参数／边界 |
|---|---|---|
| coterm_serial_driver_info | read | 无，驱动级，含本机 ports |
| coterm_serial_status | read | 无，含连接、配置、队列、shellReady 和 capabilities |
| coterm_serial_probe | read | 无，状态加可枚举端口，不打开设备 |
| coterm_serial_connect | commit | 无，使用已有配置 |
| coterm_serial_disconnect | commit | 无，停止余项并释放端口 |
| coterm_serial_read_lines | read | lines?=100，1–1000，Shell／stream 均可 |
| coterm_serial_read_selected_lines | read | 无，仅 Shell |
| coterm_serial_read_dialogue_pairs | read | count?=2，1–32，仅 Shell |
| coterm_serial_command | commit | commands，lineEnding?=cr，intervalMs?=100，waitForPrompt?，waitTimeoutMs? |
| coterm_serial_command_hex | commit | commands，intervalMs?=100 |

每批 1–64 项，解码并追加行尾后的每项 1–4096 字节，总量至多 16 KiB。intervalMs 为 0–1000；文本 waitTimeoutMs 为 0–300000，Shell 默认每项等待 5000 ms，stream 可用它定时收集有界输出。HEX 不接受等待或行尾参数，支持偶数 HEX 字符、ASCII 空白和一个开头 0x，无 Base64。

waitForPrompt 是完整可见字面行，忽略尾部空白，最长 256 UTF-8 字节，不是正则或任意子串。read_lines 保留设备文本控制序列，非法 UTF-8 替换。capabilities 明确 readLines／readSelectedLines／readDialoguePairs；共享工具目录列能力并集，stream 实例拒绝后两类。

## Local：6 个工具

| 工具 | 业务参数／效果 |
|---|---|
| coterm_local_driver_info | 无，read，驱动级 |
| coterm_local_status | 无，read |
| coterm_local_command | commands，mode?=terminal／exec，commit |
| coterm_local_read_lines | lines?=100，1–1000，read |
| coterm_local_read_selected_lines | 无，read |
| coterm_local_read_dialogue_pairs | count?=2，1–32，read |

commands 为 1–64 项，源文本总量至多 64 KiB。前台共享 PTY，后台独立 Shell。每批一条记录；人工原始按键不生成对话。read_lines 是后端逻辑行，保留 ANSI，不承诺 xterm 屏幕还原。内部原始 input、start／restart、exec／output 和回放不作为独立 MCP 工具。

## Raw TCP：8 个工具；Telnet：7 个工具

以下名称分别加 coterm_tcp_ 或 coterm_telnet_ 前缀。没有 driver_info 或 probe MCP 工具，也没有 exec／文件工具。

| 后缀 | 效果 | 参数／边界 |
|---|---|---|
| status | read | 无 |
| connect | commit | 无 |
| disconnect | commit | 无 |
| read_lines | read | lines?，默认 100，1–1000 |
| read_selected_lines | read | 无，仅 Shell |
| read_dialogue_pairs | read | count?，默认 2，1–32，仅 Shell |
| command | commit | commands，intervalMs?，lineEnding?，waitTimeoutMs?，waitForPrompt? |
| command_hex | commit | 仅 Raw TCP；commands，intervalMs?，waitTimeoutMs? |

批次 1–32 项，每项字符串 1–4096 UTF-8 字节，实际发送总量至多 16 KiB。intervalMs 默认 100，0–60000；lineEnding 默认 crlf；waitTimeoutMs 默认 5000，100–60000。文本 Shell 等提示符，stream／HEX 不等待或捕获设备结果，HEX 不接受 lineEnding／waitForPrompt。不要直接套串口的范围与默认值。

## 收据、读取与命令记录

宿主收据字段为 operationId、proposalId、status、waitExpired、output、error。status 为 accepted／awaiting_approval／dispatched／running／completed／failed／rejected／cancelled／unknown。驱动命令记录的 running／completed／cancelled／timed_out／unknown 是另一层状态，不能混为同一枚举。

命令记录至少保留 stdin、stdout、stderr、状态、退出码和截断信息。SSH／Local 另有 mode、streamMode 等字段；stream Shell 记录附带 boundary、deviceCompletion。下面仅展示 SSH 记录核心字段：

```json
{
  "operationId": "operation-1",
  "mode": "terminal",
  "status": "completed",
  "stdin": "pwd",
  "stdout": "/root\r\n",
  "stderr": "",
  "stdoutEncoding": "utf-8",
  "stderrEncoding": "utf-8",
  "streamMode": "merged",
  "exitCode": 0,
  "truncated": false,
  "error": null
}
```

PTY 无独立错误通道时合并到 stdout，stderr 为空；SSH 收到独立 ExtendedData 时后端保留，不注入主终端。SSH 字节不可解码时编码字段为 base64。Serial／网络 Shell 无退出码，exitCode 为 null，观察到提示符使用 deviceCompletion=prompt_observed，未知为 unknown。

## 等待、审批与取消

tools/call 的 params._meta["coterm/waitTimeoutMs"] 控制协议等待，0 立即返回，1–300000 指定上限，省略默认 60 秒；它不是 arguments 中的业务 waitTimeoutMs。

SSH／Serial／网络实例工具及 Local command 登记 returnOnApproval，进入审批返回 structuredContent.status=pending_approval，带 operationId／proposalId；宿主收据状态仍是 awaiting_approval。后续用原操作查询，不重提。

structuredContent 保存机器结果，content 为有界可读文本；_meta["coterm/operation"] 是收据，output 替换成大小标记，不再复制完整正文。取消 outcome 为 not_submitted／stopped／unsupported／unknown；只有前两者能据确认收敛为取消。
