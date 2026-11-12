# 内部契约与实现限制

本文件记录宿主、内置驱动和前端之间的内部控制、配置及容量约束，不作为外部可调用 API。对外工具见[MCP API](../api/mcp.md)，整体依赖见[代码架构](architecture.md)。

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

共享终端默认保留 3000 屏幕滚动行；搜索查询至多 512 字符、100 ms 防抖、最多高亮 1000 处。发送历史当前挂载内至多 50 项且合计 64 KiB；UART 定时默认 100 ms，20–60000 ms，不持久化。数值不是任意速率无损保证，实测边界见[状态](../feedback/known-issues.md)。

## 配置与内部操作

MCP 设置文件 mcp-settings.v1.json 保存 version、port、startupMode，启动方式为 auto／manual。文件上限 4 KiB，未知字段和损坏配置明确报错；运行中换端口先绑定新桥并保存，成功后才关闭旧桥。手动启停不修改下次启动方式。

配置字段为 port、baudRate、dataBits、stopBits、parity、dtr、rts、mode、prompt。后端省略 mode 默认 stream；界面新建默认 shell，提示符初值为 #，不适用设备须填写完整值。dataBits 默认 8（5–8），stopBits 默认 1（1／2），parity 默认 none（none／odd／even），dtr／rts 默认 true。

内部 configure、write／write_hex、signals、set_selection 和 read／read_hex 不作为 MCP 工具公开；人工单次写至多 16 KiB，文本默认不补行尾。signals 可在连接中单独改变 DTR／RTS，有命令批次时拒绝；这不代表硬件流控握手。每驱动最多 32 实例，同一端口不重复分配。

连接配置字段为 host、port、username、connectTimeoutMs、autoReconnect、reconnectIntervalMs、proxy、localEcho、receiveNewlines、backspace、terminalType、mode、prompt。host 为 1–253 ASCII 字节，不带协议／路径／端口；IPv6 不加方括号。Telnet username 可选，1–128 UTF-8 字节；Raw TCP 拒绝用户名。

connectTimeoutMs 默认 5000（100–60000），包含 DNS、连接和代理握手；reconnectIntervalMs 默认 1000（250–60000），autoReconnect 默认 false。proxy 是 direct 或无认证 socks5／http 的 host、port。localEcho 为 auto／on／off，receiveNewlines 为 none／lf／cr／both，backspace 为 bs／del，terminalType 为 xterm／xterm-256color／vt100。

Telnet 默认 shell、auto 回显、不补换行、DEL；Raw TCP 默认 stream、on 回显、LF 补 CR、BS。两者 Shell 默认提示符 #，需与设备完整提示符匹配。Telnet 支持服务端发起的 BINARY、ECHO、SGA、NAWS、TTYPE 协商；子协商上限 512 字节，不声明完整通用状态机。

## 对外接口实现定位

来源：[SSH 目录与工具](../../crates/coterm-driver/src/ssh/backend.rs)、[命令与结果记录](../../crates/coterm-driver/src/ssh/blocks.rs)、[连接执行](../../crates/coterm-driver/src/ssh/session.rs)。

来源：[Serial 目录](../../crates/coterm-driver/src/serial/backend.rs)、[批次](../../crates/coterm-driver/src/serial/command.rs)、[连接](../../crates/coterm-driver/src/serial/session.rs)、[提示符](../../crates/coterm-driver/src/stream/shell.rs)。

来源：[Local 目录](../../crates/coterm-driver/src/local/backend.rs)、[命令与记录](../../crates/coterm-driver/src/local/command.rs)、[Shell 解析](../../crates/coterm-driver/src/local/shell.rs)。

来源：[网络目录](../../crates/coterm-driver/src/network/catalog.rs)、[网络配置](../../crates/coterm-driver/src/network/config.rs)、[Telnet codec](../../crates/coterm-driver/src/telnet/codec.rs)。
