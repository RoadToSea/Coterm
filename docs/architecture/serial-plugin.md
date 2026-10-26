# 串口插件

实现位于 `plugins/plugin-serial`。串口的打开、参数、原始字节读写、回放缓冲和界面均归插件；基座只提供加载、实例路由、操作审批、MCP 工具目录和事件转发。

串口与 SSH 的差别在于语义：SSH 有协议层，可以测量握手、认证和命令边界；UART 只有字节。因此本插件不推断提示符、不识别命令结束、不解析设备输出，`serial.read` 返回的就是设备在这段时间发出的原始字节。

## 使用

仓库根目录执行：

```powershell
pnpm install
pnpm plugin:serial:build
pnpm tauri:dev
```

配置表单只有串口字段：Port、Baud rate、Data bits、Stop bits、Parity。默认 `115200 8-N-1`，与常见 Linux 开发板的调试串口一致。提交后基座启动插件并创建实例；在实例面板点击 Connect 打开端口。

配置阶段还没有实例，而 `serial.probe` 是实例级操作，所以配置表单里的端口名是手输或从下拉建议里选，真正的系统枚举发生在实例面板的 Ports 按钮上——那里已经有实例，可以合法调用 `serial.probe`。

同一个插件可创建多个实例，但同一物理端口不能被两个实例同时占用（端口名比较不区分大小写）；切换实例只卸载界面，不断开端口。

未实现：流控（RTS/CTS、XON/XOFF）、DTR/RTS 手动控制、波特率低于 9600、断线自动重连、端口热插拔监听、文件发送/接收、时间戳标记。`serialport` crate 目前同步阻塞，读写线程模型见下文。

## 插件操作

| 操作 | effect | transient | 语义 |
|---|---|---|---|
| `serial.status` | read | 是 | 配置、连接状态和连接 ID，不打开设备 |
| `serial.probe` | read | 是 | 枚举操作系统可见的串口，附本实例配置；不打开设备 |
| `serial.connect` | commit | 否 | 按实例配置打开端口，启动读取线程 |
| `serial.disconnect` | commit | 否 | 关闭端口，丢弃该实例未决提案 |
| `serial.read` | read | 是 | 按十进制字符串游标读取 base64 字节，报告 gap、closed、more |
| `serial.write` | commit | 是 | base64 字节加可选行结束符写入设备 |
| `serial.plugin_info` | read | 否 | 插件级只读操作，不绑定实例 |

`serial.write` 的 `lineEnding` 取 `none`、`lf`、`cr`、`crlf`，由插件解释后追加到 base64 解码结果之后；基座不感知行结束符语义。有效负载合计 1 到 16384 字节，空负载和超限都被拒绝。

`serial.connect` 与 `serial.disconnect` 会清除该实例上所有未决的写入提案：物理连接变了，针对旧连接的批准快照不再有意义。

`serial.cancel` 对任何目标都返回 `{"outcome":"unsupported"}`。串口写入是短事务，不存在需要中途停止的长操作；不确认是明确的，而不是冒充成功。

## MCP 工具

插件显式注册 `mcp.serial.status`、`mcp.serial.probe`、`mcp.serial.connect`、`mcp.serial.disconnect`、`mcp.serial.read` 和插件级的 `mcp.serial.plugin_info`。除 `plugin_info` 外每个工具都要求 `pluginInstanceId`，基座在握手期校验这一点。

**`serial.write` 不暴露为 MCP 工具。** 原始字节会直接驱动硬件——写到开发板上可能就是一条 shell 命令。Agent 可以读、可以打开和关闭端口，但不能直接向设备写字节；写路径保留给人工界面。这条边界是有意的，修改前需要先决定它由谁审批。

MCP 参数校验是严格的：未知键、非字符串值、缺失必需键都会拒绝，实例参数与操作声明的实例不一致也会拒绝。

## 线程模型

`serialport` crate 只有同步阻塞接口，不能直接放在异步运行时的工作线程上。实现分工如下：

- **读取线程**：独立的 `std::thread`，独占端口读端，按 20 ms 读超时循环 `read()`。读到数据后写入回放缓冲，并尝试向有界通知队列 `try_send`。
- **写入 actor**：一个 `tokio::spawn` 任务持有写端，`select!` 等待写请求或读取线程退出信号。每次写入移交到 `spawn_blocking` 执行，返回后再把写端放回，避免闭包捕获所有权问题，也不会长时间占住异步线程。
- **串行化**：写请求经 64 槽队列按顺序提交；队列满或关闭时明确报错，而不是静默丢弃。

读取线程是唯一决定"设备说了什么"的地方，因此回放缓冲始终是权威来源。通知队列满时 `try_send` 直接丢弃推送——慢消费者会在下一次读取时通过游标和 gap 重新对齐，不阻塞读取线程，也不影响其他订阅。

## 配置校验

`Config` 使用 `deny_unknown_fields`，拼错的字段名（例如 `databits`）会报错而不是被静默忽略为默认值。端口名必须非空、不超过 256 字节、不含控制字符、不含 `..`、`/`、`\`——端口名最终交给操作系统解析，这些检查先挡住路径穿越和控制字符注入。`baudRate` 为 0 被拒绝，`dataBits` 只接受 5 到 8，`stopBits` 只接受 1 或 2。

## 数据限制

每连接最多缓存 256 KiB 原始输出，每块最多 8 KiB，单次读取不超过 8 KiB，使用 base64 避免损坏非 UTF-8 字节。游标超出当前末尾时报错，而不是静默返回空；游标早于最老块时返回 `gap=true`，界面重置终端并提示。

界面与 SSH 一致：先安装事件订阅，再读一次状态和回放快照，随后只消费推送；连接变化或报告缺口时按游标回放，空闲时不轮询。前端输入队列不超过 16 KiB。

终端里的按键按原始字节发送，不附加行结束符；下方的发送框提供 CR / CRLF / LF / None 选择和 HEX 模式。HEX 解析在插件前端完成，基座只见 base64；奇数位或非十六进制字符直接报错。

## 事件

插件发送通用实例事件 `serial.output`（连接 ID、十进制游标、base64 字节）和 `serial.status`。基座只按插件实例转发。连接 ID 防止旧连接的事件被应用到新连接。读取线程退出、端口消失或断开时，写入 actor 把缓冲标记为 closed 并发出 `serial.status {"connected":false}`。

## 验证

```powershell
cargo test -p coterm-plugin-serial
```

单元测试覆盖配置校验（控制字符、路径、零波特率、非法数据位）、回放缓冲的缺口判定与内存上限、单次读取上限、写入负载（base64 解码、四种行结束符、1 到 16384 字节边界）、MCP 目录不含 `serial.write`，以及 MCP 参数必须显式绑定实例。

集成测试 `plugins/plugin-serial/tests/serial.rs` 覆盖两条链路：直接驱动后端验证握手目录、重复端口拒绝、路径穿越拒绝、未知配置键拒绝、未打开端口的读写失败、非法游标、提案的提交/否决/不可重批；以及经真实基座运行时加载插件包、握手校验、创建实例、`serial.probe` 枚举、MCP 实例参数不匹配被拒（以 `failed` 回执返回，不是传输层错误）。

真实硬件验证需要一台已连到本机、处于 shell 提示符的设备：

```powershell
$env:COTERM_SERIAL_LIVE = "COM6"
$env:COTERM_SERIAL_LIVE_BAUD = "115200"
cargo test -p coterm-plugin-serial --test live -- --nocapture
```

不设环境变量时该测试直接跳过，不会在没有板子的机器上失败。手工往返可用 `cargo run -p coterm-plugin-serial --example live -- COM6 115200`。

本轮在 `USB-SERIAL CH340 (COM6)` 上以 115200 8-N-1 实测：`serial.probe` 枚举到 `COM6`、`COM4`；`serial.connect` 后发送 `\r` 收到 shell 提示符，`uname -a` 返回 `Linux PingDiChan 5.4.61 #4 PREEMPT riscv64 GNU/Linux`，回放游标随真实字节推进，`serial.disconnect` 后状态归零。板子的内置 shell 已直接处于 `#` 提示符，不需要登录，所以发送 `root` 会被当作命令并回显 `-sh: root: not found`——这是设备的实际行为，不是插件错误。
