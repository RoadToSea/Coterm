# 当前接口参考

本文件是[代码架构](../05-code-architecture.md)的技术参考，不代替[模块需求](../02-requirements/)。工具目录以对应驱动登记的 schema 为准，下面记录当前公开范围与关键边界。

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

MCP 设置文件 mcp-settings.v1.json 保存 version、port、startupMode，启动方式为 auto／manual。文件上限 4 KiB，未知字段和损坏配置明确报错；运行中换端口先绑定新桥并保存，成功后才关闭旧桥。手动启停不修改下次启动方式。

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

来源：[SSH 目录与工具](../../crates/coterm-driver/src/ssh/backend.rs)、[命令与结果记录](../../crates/coterm-driver/src/ssh/blocks.rs)、[连接执行](../../crates/coterm-driver/src/ssh/session.rs)。

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

配置字段为 port、baudRate、dataBits、stopBits、parity、dtr、rts、mode、prompt。后端省略 mode 默认 stream；界面新建默认 shell，提示符初值为 #，不适用设备须填写完整值。dataBits 默认 8（5–8），stopBits 默认 1（1／2），parity 默认 none（none／odd／even），dtr／rts 默认 true。

内部 configure、write／write_hex、signals、set_selection 和 read／read_hex 不作为 MCP 工具公开；人工单次写至多 16 KiB，文本默认不补行尾。signals 可在连接中单独改变 DTR／RTS，有命令批次时拒绝；这不代表硬件流控握手。每驱动最多 32 实例，同一端口不重复分配。

来源：[Serial 目录](../../crates/coterm-driver/src/serial/backend.rs)、[批次](../../crates/coterm-driver/src/serial/command.rs)、[连接](../../crates/coterm-driver/src/serial/session.rs)、[提示符](../../crates/coterm-driver/src/stream/shell.rs)。

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

来源：[Local 目录](../../crates/coterm-driver/src/local/backend.rs)、[命令与记录](../../crates/coterm-driver/src/local/command.rs)、[Shell 解析](../../crates/coterm-driver/src/local/shell.rs)。

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

连接配置字段为 host、port、username、connectTimeoutMs、autoReconnect、reconnectIntervalMs、proxy、localEcho、receiveNewlines、backspace、terminalType、mode、prompt。host 为 1–253 ASCII 字节，不带协议／路径／端口；IPv6 不加方括号。Telnet username 可选，1–128 UTF-8 字节；Raw TCP 拒绝用户名。

connectTimeoutMs 默认 5000（100–60000），包含 DNS、连接和代理握手；reconnectIntervalMs 默认 1000（250–60000），autoReconnect 默认 false。proxy 是 direct 或无认证 socks5／http 的 host、port。localEcho 为 auto／on／off，receiveNewlines 为 none／lf／cr／both，backspace 为 bs／del，terminalType 为 xterm／xterm-256color／vt100。

Telnet 默认 shell、auto 回显、不补换行、DEL；Raw TCP 默认 stream、on 回显、LF 补 CR、BS。两者 Shell 默认提示符 #，需与设备完整提示符匹配。Telnet 支持服务端发起的 BINARY、ECHO、SGA、NAWS、TTYPE 协商；子协商上限 512 字节，不声明完整通用状态机。

来源：[网络目录](../../crates/coterm-driver/src/network/catalog.rs)、[网络配置](../../crates/coterm-driver/src/network/config.rs)、[Telnet codec](../../crates/coterm-driver/src/telnet/codec.rs)。

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

## 内部控制与前端契约

| 契约 | 内容与边界 |
|---|---|
| HostCommand | CreateInstance、DestroyInstance、Invoke、Approve、Reject、Cancel、InteractionResult、ProvideSecret、Shutdown |
| InvokeRequest | operationId、driverInstanceId、source、effect、operation、args、dispatch；dispatch 为 execute 或 prepare_proposal |
| DriverNotification | operation_completed、operation_failed 或实例 event；长操作先返回 _cotermDeferred |
| 延迟交互 | _cotermInteraction 由宿主消费，不存为普通结果；秘密通过专用投递 |
| RuntimeError | category、code、message、details；类别为 invalid_request／not_found／unsupported／conflict／interaction／transport／domain／internal |
| DriverAPI 2.0 | 绑定 driverId、driverInstanceId、sessionId；可选 instanceConfigRef 和 saveConfiguration；invoke 不允许改目标或来源 |
| subscribeEvents | 安装监听后返回注销函数，处理完成确认消费；卸载不销毁实例 |
| host_attention | version、approvals、interactions；host://attention-changed 只发送版本，不带参数和秘密 |

字段为 camelCase，标签为 snake_case。普通领域结果不占顶层 _coterm* 保留名。显式诊断适配器将这些控制动作映射成逐行 JSON-RPC，解析错误使用标准码、业务错误为 -32000；它不参与产品驱动控制。

规则组技术字段为 id、name、mode、patterns；模式为 allow_matches／approve_matches。实例保存 approvalRuleGroupId 和 bypass。命令 schema 的 x-coterm-approval-text 指向 /commands 或内部 /text／/script；未声明、无效指针或错误类型不获规则豁免。Rust regex 不支持环视和反向引用，默认任意位置匹配，不解释 Shell 语法。

## 容量与保留

| 对象 | 当前边界 |
|---|---|
| 驱动控制 | 64 队列，单次 JSON 最大 1 MiB，控制应答 10 秒超时关闭驱动任务 |
| 完成／事件 | 完成队列 64，事件广播 64；完成溢出关闭任务，事件遗漏报告缺口 |
| 桌面事件订阅 | 至多 32，每订阅 64 队列，30 秒未确认关闭 |
| 宿主普通／transient 账本 | 各每实例 100 条，输入 16 KiB，输出 192 KiB，保护非终态；仅本次启动 |
| 交互 | 至多 32；秘密句柄 30 秒、一次消费、绑定实例与用途 |
| 工作区 | 128 保存项，32 组，明文编码至多 512 KiB；Windows DPAPI，不含操作历史 |
| 规则组 | 至多 128 组，每组 128 规则，单条 2048 UTF-8 字节，规则正文总量 16 KiB |
| SSH 历史 | 普通与独立错误流各 256 KiB；命令至多 32 条，源输入及每输出流各 64 KiB，结果快照至多 96 KiB |
| SSH 显示预览 | 每命令预览 1 KiB，输出预览合计 2 KiB，块页 12 KiB，选区返回 16 KiB |
| Local 历史 | PTY 256 KiB，32 记录，每流 16 KiB，JSON 快照 12 KiB；后台 8 作业，每流原始缓冲 64 KiB |
| Serial／网络接收 | 每连接 256 KiB，内部按 1 KiB 块回放；Shell 最近 32 对，每项输出 16 KiB，对话 JSON 12 KiB |
| Serial 命令／预览 | 最多 16 在途批次，写入 actor 64 槽，前端输入 16 KiB；回显预览 JSON 文本 6 KiB |
| 网络输入／读取 | 每实例输入队列 32，驱动通知 128，socket 单次读取至多 64 KiB，不等待填满 |

Serial 系统读取至多 1 KiB，按 1 KiB／4 ms／空闲合并通知；Windows 读间隔 1 ms、总等待 20 ms、写期限 2 秒，读写独立。打开等待 5 秒，关闭读线程等待 2 秒。系统 flush 明确不支持时才有界查询发送排空；失败不重发。

网络大事件内部游标按 ceil(bytes / 1024) 前进，不能固定按一个事件一步判断连续。只对前端回放使用这些位置，MCP 不公开游标。

共享终端默认保留 3000 屏幕滚动行；搜索查询至多 512 字符、100 ms 防抖、最多高亮 1000 处。发送历史当前挂载内至多 50 项且合计 64 KiB；UART 定时默认 100 ms，20–60000 ms，不持久化。数值不是任意速率无损保证，实测边界见[状态](../status.md)。
