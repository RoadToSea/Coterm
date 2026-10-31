# 串口插件

实现位于 [plugin-serial](../../plugins/plugin-serial)。插件提供分离的通用文本与 HEX 收发，不解析设备提示符、不推断命令完成或退出码。Modbus 等设备协议与固件烧录由独立插件实现，本插件不组协议帧、不计算 CRC、不实现下载握手。

## 使用

按 [README](../../README.md) 安装依赖后，执行 `pnpm plugin:serial:build` 和 `pnpm tauri:dev`。

填写 Port、Baud rate、Data bits、Stop bits、Parity、DTR、RTS 创建实例，默认 115200 8-N-1、DTR 与 RTS 置高；在实例面板 Connect。配置阶段手填端口，创建实例后可用 Ports 枚举。实例切换不关闭连接，同一端口不能被多个实例占用。

端口名按平台规则校验：Windows 接受 `COM6` 这类设备名（不区分大小写），Unix 接受 `/dev/...` 设备节点；其余带分隔符的名字一律拒绝，避免把配置变成任意文件打开入口。部分开发板会因 DTR 变化复位，可为这类板子取消「打开时置高 DTR」。

人工 Shell 模式与 SSH 一样直接在终端中交互，由设备负责回显与编辑，不另设命令发送框或列表编辑器。两者采用同一套[终端快捷键](terminal-shortcuts.md)：Ctrl+C 有选区时复制、无选区时发送中断；Ctrl+F 搜索当前终端输出，Enter/Shift+Enter 跳转、Esc 关闭；粘贴、终端全选和清除选区在本地处理，其余 Shell 按键沿用终端语义。切换视图不重建连接，也不发送命令。命令列表是 MCP 面向 Agent 的批量接口，不改变人工交互方式。

UART 调试模式使用发送框；Enter 发送，Shift+Enter 换行。文本模式使用 `text` 和可选 none/lf/cr/crlf 行结束符；勾选「按 HEX 输入」后仅使用 `hex`，禁用行结束符，不隐式附加字节。HEX 接收视图才调用二进制读取接口，文本视图和 Shell 使用文本读取与事件。两种模式保留有界定时重复发送。单次写入总量为 1～16384 字节，HEX 必须为偶数位有效十六进制字符，可包含 ASCII 空白和一个开头 `0x`。MCP 列表发送期间禁止人工写入及信号修改，避免混写。

## 操作与 MCP

| 操作 | effect | 用途 |
|---|---|---|
| `serial.status/probe` | read | 状态与系统端口枚举 |
| `serial.connect/disconnect` | commit | 打开或关闭端口，清理该实例旧提案 |
| `serial.read` | read | `text` 游标回放，报告 bytes/cursor/gap/closed/more；`cursor` 是从 0 开始的非负整数 |
| `serial.get_recent` | read | 无状态读取有界输出缓冲的最近文本行，默认 50 行；适合不想保存游标的调用方 |
| `serial.write` | commit | 人工文本输入 `text`，保留控制字符；标记为敏感输入 |
| `serial.command` | commit | Agent 文本输入列表 `commands`，与人工 Shell 同样按 UTF-8 原样发送；可选 `waitForPrompt` 和 `waitTimeoutMs` 有界收集新输出 |
| `serial.read_hex` | read | 显式 HEX 读取，返回 `hex` 与相同整数游标、状态和原始字节计数 |
| `serial.write_hex` | commit | 人工 HEX 输入 `hex`，没有行结束符或文本参数 |
| `serial.command_hex` | commit | Agent HEX 字符串列表 `commands`，共用有界 FIFO 和审批 |
| `serial.signals` | commit | 设置 DTR/RTS 电平 |
| `serial.plugin_info` | read | 插件级信息 |

MCP 提供 status、probe、connect、disconnect、read、get_recent、read_hex、command、command_hex、plugin_info，不开放人工单次 write/write_hex 与 signals。除 plugin_info 外使用 `name` 指定 `coterm.list_instances` 返回的唯一实例名称，桥接层也兼容 `instance`、`instanceName` 与 `instance_name` 别名，UUID 由基座内部路由。文本接口和 HEX 接口互不接受对方参数。`serial.command` 与人工 Shell 共用文本编码逻辑，允许 Ctrl+C (`\u0003`)、Tab、方向键 ESC 序列、换行、DEL、NUL 等所有控制字符；默认 `lineEnding:"cr"`，面向 Shell 的命令会直接提交；需要原样输入时显式指定 `none`。列表只是顺序发送的输入片段，无单行或可打印字符限制。人工输入不用列表。Agent 来源的提交经过基座审批，除非用户主动开启 Bypass 或明确放行；列表整体为审批快照，输入在普通账本中脱敏，进度事件不包含输入内容。

Base64 和 HEX 都是字节的表示方式，均不是加密。本插件接口、事件与前端已移除 Base64；文本直接使用 JSON 字符串，二进制通过显式 HEX 工具收发。本次变更不改变基座私有 Bridge 的认证或传输机制。

例如调用 MCP `serial.command`：

```json
{
  "name": "COM1@115200",
  "commands": ["cd /tmp", "pwd", "ls -la"],
  "lineEnding": "cr",
  "intervalMs": 100
}
```

原样输入和 HEX 输入示例（以下为各自工具的 arguments，不混用）：

```json
{"name":"COM1@115200","commands":["\u0003","pwd\r","\u001b[A\r"],"lineEnding":"none","intervalMs":100}
```

```json
{"name":"COM1@115200","commands":["00 80 FF","0d0a"],"intervalMs":100}
```

第二个示例调用 `coterm-plugin-serial__serial.command_hex`，返回读取使用 `coterm-plugin-serial__serial.read_hex`，其载荷形如 `{"hex":"0080ff0d0a","bytes":5,"cursor":2,"gap":false,"closed":false,"more":false}`。不自动追加换行，也不把 `0d0a` 当四个文本字符发送。

每个列表为 1～64 项，每项实际发送 1～4096 字节（包含显式选择的行结束符）；文本列表允许空字符串，但必须选择 `cr`、`lf` 或 `crlf` 使实际载荷非空。整批最多 16 KiB，每实例最多 16 个在途列表。文本与 HEX 列表共用同一 FIFO，一个列表的项不会与其他列表交叉；整批校验失败时不会发送任何项。有在途列表时人工文本、HEX 写入和 DTR/RTS 修改均拒绝。不会拼接 `&&`、`eval` 或 shell 包装脚本。

请求入队后通过运行时 DeferredOperation 和通知完成，控制通道保持可响应读取、状态与取消。结果包含 `total/sent/remaining/bytes`、`deviceCompletion:"unknown"`；MCP 返回在 `structuredContent` 提供完整机器可读结果，同时文本块保留发送摘要及等待期间捕获的输出。`serial.command` 进度事件额外携带操作 ID、连接 ID、累计发送字节数，`serial.status.commandQueue` 可恢复在途列表数。**完成只表示所有字节写入并 flush 成功，不代表设备执行完成**：不会推断退出码，也不因设备命令失败而自动停止。可传 `waitForPrompt`（区分大小写的字面子串，不是正则）或 `waitTimeoutMs`（0～300000，提示符等待默认 5000；只传超时表示定时收集），在发送后有界收集至多 8 KiB 新输出并返回 `output.text/output.cursor/output.promptMatched/output.truncated/output.gap`；超时不代表设备失败。未提供等待字段时仍只返回发送收据。登录密码和交互式程序仍需人工确认设备状态。

取消只针对列表：队列中的列表可返回 `not_submitted`；已经开始的列表等待当前写入确认后停止余下项，返回 `stopped`。无法确认时返回 `unknown`。原始字节写入及其他操作的取消仍为 `unsupported`。断开、读写故障或写入超时会丢弃尚未发送的项并报告未知交付，包含已确认发送的项数；当前项可能部分送达，不自动重放，重新连接后也不恢复旧队列。

串口 MCP 工具通过注册 descriptor 的 `_meta["coterm/returnOnApproval"]: true` 选择审批就绪即返回模式：首次调用返回 `structuredContent.status:"pending_approval"`、操作 ID 和提案 ID，操作收据仍使用运行时的 `awaiting_approval`。用户批准后用 `coterm.get_operation` 查询原操作；不要再次提交同一命令。未需审批时继续等待正常完成；`coterm/waitTimeoutMs:0` 仍立即返回。等待输出期间也支持取消和断开，不会因最长 300 秒的收集期限阻塞取消。

不保存游标时可调用 `serial.get_recent`，例如 `{"name":"COM1@115200","lines":50}`。`lines` 范围为 1～200，默认 50；返回 `text/lines/cursor/connectionId/closed/gap/truncated`。这是缓冲尾部的快照，重复调用可能包含相同内容；需要精确增量时仍用 `serial.read` 的独立游标。等待回显和近期快照均另设 6 KiB 的 JSON 转义文本预算，保证文本块与结构化副本一起放进宿主 16 KiB 结果上限；达到上限时标记 `truncated`，不会把超限结果静默替换成摘要。控制字符较多时实际文本字节数会少于此预算。

## 数据与已知限制

- 每连接缓存 256 KiB 原始数据及有界文本副本，每块及单次读取最多 1 KiB 原始数据；为 JSON 控制字符转义和 MCP 信封预留空间。游标过期报告 gap，超出末尾报错。
- 文本/HEX 读取共用连接 ID 和块游标，消费者独立保存游标。文本增量解码保留跨读取的 UTF-8 尾字节，非法编码用替换字符表示，连接关闭时结束解码；HEX 始终精确保留原字节。
- `serial.output` 事件只含 `text/bytes/cursor/connectionId`，不携带 Base64 或 HEX 副本；通知队列有界，推送丢失后以回放缓冲恢复。HEX 视图按事件触发 `serial.read_hex`，退出 HEX 视图后停止调用二进制读取接口。
- 读取使用独立线程，写入经 64 槽队列和阻塞任务提交；前端输入队列上限 16 KiB。
- 命令列表共用写入 actor，每项单独写入和 flush；每项驱动等待上限 2 秒，间隔等待可取消。断开前先停止后续项，再回收连接；Windows、Linux、macOS 共用该实现，无新增依赖、权限或传输方式。
- 部分虚拟串口驱动不实现 flush（Windows 错误码 1/50）。仅当驱动明确不支持该操作时，写端改为有界查询发送队列，确认排空后才计入发送成功；队列查询失败或排空超时仍为错误，不重发数据。原始写入和列表发送共用这一兼容逻辑。列表失败的结构化信息包含 IO 阶段、错误种类和 OS 错误码，不包含命令内容。
- 读取线程有停止标志，显式断开时等待其退出并回收句柄（上限 2 秒）；析构也会置位该标志，避免线程泄漏。停止标志只在一次阻塞读之后检查，最坏退出延迟是一次读取轮询（20 ms）。
- 打开端口是阻塞调用，放在阻塞线程池执行；5 秒超时后关闭端口并报错，不占用异步工作线程。
- `serial.status` 回报 DTR/RTS 电平与本次连接累计收发字节数；前端 TX 计数取插件回报的实际写入字节，包含插件补上的行结束符。
- 文本与 HEX 写入及列表均标记为敏感输入，账本只保留脱敏占位。
- 尚无硬件流控（RTS/CTS 握手）、自动重连、热插拔监听和文件收发。

公共交互、事件和账本规则见 [运行时通信](runtime-communication.md)，修复优先级见 [项目进度](../progress/project.md)。

## 验证

普通契约与基座加载测试不打开或写入物理端口：

```bash
cargo test -p coterm-plugin-serial
```

MCP 列表回归使用内存写端验证顺序、独立写入、取消、故障后停止和字节计数；契约测试验证审批前整批校验及 MCP 实例绑定，不向物理端口写入。MCP 列表在真实设备上的发送节奏仍需按设备接收能力验收。

无需硬件的人工交互浏览器验收（需要 Chrome/Edge/Chromium，使用实际前端构建和宿主 API 替身，验证 Shell 按键直发、UART 发送框及 MCP 队列占用期间的输入隔离）：

```bash
pnpm --filter coterm-plugin-serial-ui build
node --import ./scripts/test-ts-loader.mjs --test tests/serial-terminal-browser.test.mjs
```

硬件用例默认 ignored。确认目标设备可接收写入后显式执行，将示例端口替换为实际端口：

```powershell
$env:COTERM_SERIAL_LIVE = "COM6"
$env:COTERM_SERIAL_LIVE_BAUD = "115200"
cargo test -p coterm-plugin-serial --test live -- --ignored --nocapture
```

该用例验证真实字节往返、收发计数与 DTR/RTS 落地，不验证设备业务语义；缺少设备配置会失败。手工诊断可用 `cargo run -p coterm-plugin-serial --example live -- COM6 115200`，不属于普通回归。

### 虚拟串口配对

无需登录设备 shell。先创建一对互联且未被占用的虚拟串口，再显式选择两端；测试会向这两个端口发送测试数据，默认 ignored，普通回归不会自动打开它们：

```powershell
$env:COTERM_SERIAL_PAIR_A = "COM1"
$env:COTERM_SERIAL_PAIR_B = "COM2"
$env:COTERM_SERIAL_PAIR_BAUD = "115200"
cargo test -p coterm-plugin-serial --test virtual_pair -- --ignored --nocapture
```

测试启动真实插件进程，经宿主来源识别、审批、stdio 控制协议和 OS 串口驱动验证列表发送、反向二进制回传、取消、容量、断开不重放与重连。可交换 A/B 再测；它不验证设备命令执行、提示符识别或物理线路时序。Windows HHD COM1/COM2 验收结果及驱动兼容修复见[串口开发记录](../progress/serial-plugin.md)。

界面验收在真实 Tauri 中运行，需要可枚举的串口（默认 `COM6`，可用 `COTERM_SERIAL_LIVE_PORT` 覆盖）：

```bash
pnpm plugin:serial:build
pnpm --filter coterm-desktop dev   # 另一个终端保持运行
node scripts/test-serial-ui.mjs
```

脚本会连接开发板并按需登录（`COTERM_SERIAL_USER`/`COTERM_SERIAL_PASSWORD`，默认 `root`/`root`），随后验证终端与 UART 两种视图、UTF-8 与 HEX 渲染、收发计数、DTR/RTS 切换、语言切换保持连接，以及断开后的干净状态。它会向物理设备写入字节。
