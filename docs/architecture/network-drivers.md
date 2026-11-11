# Telnet 与 Raw TCP 驱动

Telnet（`coterm-driver-telnet`）与 Raw TCP（`coterm-driver-tcp`）是 `coterm-driver` 内部模块，后端和前端随桌面编译分发，不启动驱动进程。每实例独立保存配置、连接、审批引用、命令状态和接收缓冲。

## 模块与数据流

```text
人工终端 / 字节调试                 外部 Agent
          ↓                          ↓ MCP
          └── Host API / 来源识别 / 实例审批 ──┘
                             ↓ HostCommand
            tcp::Backend / telnet::Backend
                             ↓
            network：配置、提案、命令、连接生命周期
                             ↓ 有界输入队列
            每实例 TCP 任务 → 直连 / SOCKS5 / HTTP CONNECT
                             ↕
            Telnet codec（仅 Telnet） / 原始字节（Raw TCP）
                             ↓
            stream::OutputBuffer + ShellHistory
                    ↙                    ↘
           MCP 后端快照             实例事件 / 内部回放
                                        ↓
                         公共工作台的前端显示副本
```

- `src/tcp` 和 `src/telnet` 提供两个固定入口，`src/network` 复用 TCP 连接生命周期；Telnet 的 IAC/NVT 处理位于 `src/telnet/codec.rs`。
- `src/stream` 提供串口与网络共同使用的有界字节缓冲、UTF-8／HEX 编码和提示符观察。网络不调用串口协议实现，也不依赖串口控制线或帧格式。
- `apps/shared/components/terminal-surface.ts` 管理 xterm、主题、字号、搜索、快捷键和尺寸监听；SSH、串口、Raw TCP、Telnet 共用。
- `apps/shared/components/stream-workbench/` 管理终端／字节视图、回放、选区、发送队列、HEX、定时发送、历史和提示；串口与网络通过实际连接参数适配，不传伪造 UART 配置。
- `network-connection.ts` 提供两个网络驱动的配置表单。Telnet 隐藏字节面板及其工具栏，不提供 SSH 的 exec、文件和后台作业界面。

## 配置

配置使用宿主现有的完整驱动配置保存入口，不增加工作区版本或独立保存文件。创建、恢复和挂载界面不自动连接；编辑通信参数后，在下次显式连接前校验、应用并保存。保存失败阻止连接并允许重试。连接／重连期间须先断开才能改连接参数；Raw TCP 在线切换终端／字节视图只修改 `mode/prompt`。

| JSON 字段 | Telnet 默认 | Raw TCP 默认 | 边界／含义 |
|---|---|---|---|
| `host` | 必填 | 必填 | ASCII 主机名或 IP，1–253 字节；不带协议、路径、端口，IPv6 不加方括号 |
| `port` | 23 | 8080 | 1–65535，均可修改 |
| `username` | null | null | 仅 Telnet 可设置；1–128 UTF-8 字节，无控制字符 |
| `connectTimeoutMs` | 5000 | 5000 | 100–60000；统一覆盖 DNS、建连和代理握手 |
| `autoReconnect` | false | false | 只在曾成功连接后断线时重连，初次连接失败不重试 |
| `reconnectIntervalMs` | 1000 | 1000 | 250–60000；连续失败按指数退避，最多 60 秒 |
| `proxy` | direct | direct | 见下方结构；代理认证不支持 |
| `localEcho` | auto | on | `auto/on/off`；auto 根据 Telnet ECHO 协商决定，Raw TCP 无协商，auto 等价本地回显 |
| `receiveNewlines` | none | lf | `none/lf/cr/both`；仅显示侧补全 CR/LF，不改后端数据 |
| `backspace` | del | bs | 仅映射未修饰的物理退格键，粘贴内容不改写 |
| `terminalType` | xterm | xterm | `xterm/xterm-256color/vt100`；仅 Telnet 在对端请求 TTYPE 时上报，Raw TCP 无终端类型协商 |
| `mode` | shell | stream | Telnet 界面为终端；Raw TCP 通过终端／字节视图切换 Shell／stream |
| `prompt` | `# ` | `# ` | Shell 完整可见提示符，1–256 UTF-8 字节，无控制字符；匹配忽略末尾空白，不能只填写较长提示符的子串 |

```json
{"type":"direct"}
{"type":"socks5","host":"proxy.example","port":1080}
{"type":"http","host":"proxy.example","port":8080}
```

HTTP CONNECT 与 SOCKS5 只建立无认证的 TCP 隧道，不使用系统代理或执行代理程序。HTTP 响应头上限 8 KiB，SOCKS5 支持 IPv4、IPv6 与代理端域名解析；握手不吞掉目标首包。无 TLS，代理隧道也不为 Telnet／Raw TCP 提供加密。

Telnet 观察到以 `login:`／`username:` 结尾的提示后，每次连接最多自动填入一次预设用户名。密码只在终端输入，不作为连接配置保存。不识别任意设备登录脚本，也不自动执行命令。

串口既有显示偏好的存储标识和版本保持不变。网络换行、退格、回显等设置属于网络配置；接收格式、时间戳、滚动等显示选项按驱动命名空间和保存项标识隔离。发送文本／HEX 选择、定时间隔、输入历史和数据内容不写入保存配置。

## 操作与读取

两个命名空间分别是 `tcp.*` 与 `telnet.*`。公共生命周期为 `status/probe/connect/disconnect/configure`；`probe` 只返回本地状态，不打开网络。`status` 返回配置、`connected/reconnecting/connectionId/remoteEcho`、`rxBytes/txBytes/commandQueue/shellReady/lastError` 和 `capabilities`。

| 操作 | 参数 | 返回／限制 |
|---|---|---|
| `read_lines` | `{lines?:1..1000}` | 后端最近若干行，含 `stderrLines:[]/streamMode:"merged"/closed/truncated/dropped`，无需游标 |
| `read_selected_lines` | `{}` | 仅 Shell；返回 Human 明确选择的有界文本和截断标记 |
| `read_dialogue_pairs` | `{count?:1..32}`，默认 2 | 仅 Shell；最近对话在前，保留 `stdin/stdout/stderr/exitCode/boundary/deviceCompletion` |
| `command` | `{commands,intervalMs?,lineEnding?,waitTimeoutMs?,waitForPrompt?}` | 审批后按顺序发送；Shell 逐条等待提示符，stream 只确认发送 |
| `tcp.command_hex` | `{commands,intervalMs?,waitTimeoutMs?}` | HEX 解码后发送；不补行尾、不解释提示符，不生成对话 |
| `write/write_hex` | `{connectionId,text,lineEnding?}`／`{connectionId,hex}` | Human 专用，单次 1–16384 字节；不注册为 MCP 工具 |
| `set_selection` | `{connectionId,text,truncated?}` | Human 专用，选区最多 16 KiB |
| `resize` | `{connectionId,cols,rows}` | Human 专用；1–500 列、1–300 行；Telnet 在 NAWS 协商启用后发送 |
| `read/read_hex` | `{cursor?:number}` | 前端内部有界回放；MCP 不暴露游标 |

命令列表 1–32 项，每项字符串 1–4096 UTF-8 字节，整个批次发送量最多 16 KiB；`intervalMs` 默认 100，范围 0–60000；`lineEnding` 默认 CRLF。`waitTimeoutMs` 默认 5000，范围 100–60000，仅控制 Shell 提示符等待；stream／HEX 不等待或捕获设备结果，接收数据用 `read_lines` 查询。`waitForPrompt` 仅 Shell 文本命令可用。

准备提案只校验并保留配置版本／连接 ID，不建立连接或发送。返回完整草稿给宿主待审批内存，普通账本和 Agent 读取仍由宿主脱敏。批准时复核参数、实例、配置版本和连接 ID；配置变更或重连后的旧命令提案拒绝执行。每实例只允许一个命令批次，人工输入不会与批次穿插。

Shell 只以完整字面提示符观察边界。先通过人工登录到 Shell，并让后端收到当前提示符，再提交命令。人工按键不直接作为命令来源保存；仅在已确认提示符后的设备回显可识别时，记录观察到的人工对话。本地回显只写前端，不进入后端接收数据或对话记录。

`stdin/stdout/stderr` 均保留；两个传输都没有独立 stderr，合并内容在 stdout，stderr 为空，exitCode 为 null。看到提示符不等于设备成功执行；正常 Shell 结果标记 `deviceCompletion:"prompt_observed"`，stream／HEX 和不确定结果为 `"unknown"`。错误附带有界部分结果、已确认发送字节／条数，不自动补发剩余命令。

MCP 工具为 `coterm_tcp_{status,connect,disconnect,read_lines,read_selected_lines,read_dialogue_pairs,command,command_hex}` 与对应的 `coterm_telnet_*`（无 command_hex）。共享目录列出能力并集，stream 实例明确拒绝选区和对话读取；以目标 `status.capabilities` 判断实际能力。外部使用实例名称，内部使用宿主绑定的 ID；Agent 不可通过人工输入接口绕过审批。

## 连接、缓冲与故障

- 每实例 TCP 任务拥有 socket，输入队列 32 项；每驱动通知队列 128 项，输出事件过载可丢弃，前端按游标回放，不能无限堆积。
- 后端接收缓冲上限 256 KiB，按 1 KiB 块回放，UTF-8 解码跨包保留不完整字符；Raw TCP 的 HEX 路径保持任意字节，包括 NUL／0x80／0xFF。文本读取是有损 UTF-8 展示，不是二进制协议解码。
- Shell 最近 32 条记录，每条输出最多 16 KiB；行、对话、选区和错误结果均有返回大小限制，截断和淘汰显式标记。
- `connect/command` 立即返回既有 DeferredOperation 信封，完成通过通知收敛，不占用整个控制等待期限。socket 写等待 2 秒，调用结果等待 3 秒；失败后不能据超时判断未发送。
- 断线保留本次后端读取快照。重连成功使用新 connectionId 并开始新的接收缓冲，不跨连接拼接命令；旧连接的排队输入和审批命令拒绝发送。切换解释模式后，重连继续采用新模式。
- 自动重连不重放按键、命令或定时发送。手动断开、取消连接和销毁实例停止重连；取消批次不声称撤回已经发送的设备操作。
- 前端正常依赖事件，另每 2 秒补取网络状态和遗漏回放，避免丢失最后一条断线事件后保持绿色；补取串行执行，卸载注销事件、计时器和终端。

Telnet 支持 BINARY、ECHO、SGA、NAWS 和 TTYPE 的服务端发起协商；不主动发起选项请求，不宣称完整通用 Telnet 客户端。IAC 转义和子协商增量处理，单个子协商上限 512 字节；非二进制模式按 NVT 处理 CR NUL、CR LF，人工 Enter 发送 CRLF。协商帧不进入接收缓冲、字节计数或 MCP 读取。

协议依据：[RFC 854](https://www.rfc-editor.org/rfc/rfc854)、[ECHO](https://www.rfc-editor.org/rfc/rfc857)、[NAWS](https://www.rfc-editor.org/rfc/rfc1073)、[TTYPE](https://www.rfc-editor.org/rfc/rfc1091)、[SOCKS5](https://www.rfc-editor.org/rfc/rfc1928)。

回环、协议、浏览器和原生验收入口及未覆盖环境见 [网络驱动进度](../progress/network-drivers.md)。
