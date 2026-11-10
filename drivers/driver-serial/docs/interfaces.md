# Serial 接口设计

## 设计边界与调用入口

驱动负责串口连接、文本和原始字节收发、输出缓存及实例内的发送队列。它不解析设备协议、不组 Modbus 帧、不计算 CRC、不实现固件下载握手，也不推断远端命令退出码。`waitForPrompt` 仅匹配字面子串。

| 调用方 | 入口与路由 | 参数与结果约定 |
|---|---|---|
| 驱动前端 | 基座 `api.invoke(operation, args)`；宿主绑定当前实例 | 业务参数不携带实例名称；返回通用操作收据，业务结果在 `output` |
| MCP 客户端 | `coterm_serial_*` | 实例工具通过公共名称 `name` 路由；唯一确认已连接且无未知状态时可省略，`driver_info` 不绑定实例；内部操作仍为 `mcp.serial.*` |
| 驱动进程 | 宿主通用生命周期、操作和通知协议 | `initialize/create_instance/destroy_instance/invoke/approve/reject/cancel/shutdown` 沿用基座协议，不作为 Serial MCP 工具开放 |

显式 `name` 必须取自 `coterm.list_instances`，不能传内部 UUID。Serial 的 status 已声明连接状态，只在恰有一个确认已连接的串口实例、无未知状态且快照未超限时允许省略 `name`；未连接实例的 probe / connect 仍显式指定名称。Bridge 兼容 `instance`、`instanceName`、`instance_name`，一次调用只能提供一种名称字段；公开 schema 的规范字段仍是 `name`。参数采用严格 schema，未知字段会被拒绝。旧 `coterm-driver-serial__serial.*` 仅保留为未列出的兼容调用名称；完整默认路由与健康契约见 [MCP 接入](../../../docs/architecture/mcp-integration.md)。

## 操作总表

共 18 个业务操作，其中 10 个向 MCP 开放。下表省略 MCP 实例工具公共参数 `name`；`?` 表示可选。MCP 实际名称将表中 `serial.` 替换为 `coterm_serial_`，例如 `serial.command_hex` → `coterm_serial_command_hex`。

| 操作 | MCP | effect | 业务参数 | 业务结果 / 用途 |
|---|:---:|---|---|---|
| `serial.driver_info` | 是 | read | 无 | `driverId / instanceCount / ports`；驱动级操作，不绑定实例，无实例时也可枚举本机端口；`ports` 项结构同 probe |
| `serial.status` | 是 | read | 无 | `config / connected / connectionId / signals / rxBytes / txBytes / commandQueue / shellReady / capabilities` |
| `serial.probe` | 是 | read | 无 | status 结果及 `ports`；端口项含 `portName / portType`，USB 类型可带 VID/PID、厂商、产品与序列号 |
| `serial.connect` | 是 | commit | 无 | 按实例配置打开端口，返回 `connected:true`；已连接时拒绝重复打开 |
| `serial.disconnect` | 是 | commit | 无 | 关闭端口并丢弃剩余队列，返回 `connected:false` |
| `serial.read` | 否 | read | `cursor?: integer`，默认 `0` | 前端文本回放：`text / bytes / cursor / gap / closed / more / connectionId` |
| `serial.get_recent` | 否 | read | `lines?: integer`，默认 `50`，范围 `1–200` | 内部兼容的尾部文本快照；Agent 使用 `read_lines` |
| `serial.read_hex` | 否 | read | `cursor?: integer`，默认 `0` | 前端 HEX 回放；字段同 read，以 `hex` 替代 `text` |
| `serial.read_lines` | 是 | read | `lines?: integer`，默认 `100`，范围 `1–1000` | 后端尾部逻辑行：`lines / stderrLines / streamMode / closed / dropped / truncated`，无需游标 |
| `serial.read_selected_lines` | 是，限 Shell | read | 无 | 人工 xterm 选中文本：`text / truncated`；未选中时为空串 |
| `serial.read_dialogue_pairs` | 是，限 Shell | read | `count?: integer`，默认 `2`，范围 `1–32` | 最近对话，最新在前：`pairs / dropped / truncated` |
| `serial.set_selection` | 否，限 Shell | commit | `connectionId / text / truncated?` | 人工界面更新有界选中快照；连接不匹配时拒绝 |
| `serial.command` | 是 | commit | `commands: string[]`，及下表可选参数 | 按序发送文本列表，可捕获回显；返回发送进度和可选 `output` |
| `serial.command_hex` | 是 | commit | `commands: string[]`、`intervalMs?` | 按序发送 HEX 列表；不补换行，不接受文本行结束符或等待回显参数 |
| `serial.write` | 否 | commit | `text: string`、`lineEnding?`，默认 `none` | 人工单次文本写入，返回 `submitted / bytes` |
| `serial.write_hex` | 否 | commit | `hex: string` | 人工单次 HEX 写入，返回 `submitted / bytes` |
| `serial.signals` | 否 | commit | `dtr?: boolean`、`rts?: boolean`，至少一个 | 仅修改给定信号，返回修改后的 `dtr / rts` |
| `serial.configure` | 否 | commit | `baudRate? / dataBits? / stopBits? / parity? / dtr? / rts? / mode? / prompt?`，至少一个 | 断开后更新通信及解释参数；返回完整状态，不打开端口 |

`read` 操作不触发写入审批。Agent 来源的 `commit` 操作默认经过宿主审批；用户主动开启 Bypass 或明确放行后可跳过提交审批，协议自身的认证与人工交互仍保留。`write/write_hex/signals/configure` 未注册 MCP descriptor，人工界面通过基座调用。

## 命令列表参数与编码

| 参数 | 类型 | 必填 | 默认值 | 规则 |
|---|---|:---:|---|---|
| `commands` | string[] | 是 | — | 1–64 项；文本列表按 UTF-8 编码，HEX 列表解码为原字节 |
| `lineEnding` | string | 否 | `cr` | 仅文本列表；支持 `none/cr/lf/crlf`，追加到每一项末尾 |
| `intervalMs` | integer | 否 | `100` | 文本和 HEX 列表均支持，范围 `0–1000 ms`，只在两项之间等待 |
| `waitForPrompt` | string | 否 | 使用 Shell 实例的 `prompt` | 仅 Shell 文本列表；覆盖本批次的完整可见提示符行，忽略尾部空白，最长 256 UTF-8 字节，不是正则或任意子串 |
| `waitTimeoutMs` | integer | 否 | Shell 为 `5000` | 范围 `0–300000 ms`；Shell 是每条命令等待提示符的上限，stream 中表示发送后定时收集输出 |

stream 未指定等待时，命令结果仅确认发送，不创建对话。Shell 先确认已观察到实例提示符，再逐条发送、逐条等待；当前条超时或取消后不再发送余项，迟到的提示符不会重放队列。每项实际载荷为 1–4096 字节，包含追加的行结束符；整批最多 16 KiB。文本空字符串在追加行结束符后非空时有效。整批先校验，任何一项不合法时均不发送。

文本保留 Ctrl+C、Tab、ESC、换行、DEL、NUL 等控制字符；原样输入必须显式设置 `lineEnding:"none"`。人工 `serial.write` 默认不追加结束符，与列表默认 CR 不同。

HEX 接受偶数位十六进制字符，可包含 ASCII 空白和一个开头 `0x`，不隐式追加任何字节。文本与 HEX 参数不能混用，不支持 Base64。人工单次写入总量为 1–16384 字节。

## 实例配置与生命周期

配置在创建实例时提供；`serial.connect` 本身不接收端口参数。

`serial.status.connected` 表示本机串口连接仍处于打开且未停止的状态；读写故障或显式关闭一旦标记停止，立即报告 false，不等待异步任务发送最终事件或完成收尾。页面通过同一状态字段更新连接开关。此字段不证明远端开发板在线：USB 转串口仍插着、仅断开另一端接线时，端口仍可能正常打开；不通过无数据、DTR / RTS 电平或未识别到提示符推断断线。

`serial.configure` 只在断开状态接受非空参数补丁，字段约束同下表（波特率上限为 u32 最大值）；未知字段、空补丁和类型错误均拒绝。端口身份固定，不能通过该接口更换 `port`。成功后清理该实例的旧提案，并发布 `serial.status`；下一次连接使用新参数。它只更新当前驱动进程内的实例配置，不改宿主的初始配置快照或已有实例名称，重建 / 恢复实例仍使用宿主保存的初始参数。

人工界面只在断开时维护参数草稿，用户点击连接后按 `serial.configure` → `serial.connect` 顺序执行；编辑本身不触发后端调用，配置失败不会继续连接。连接中的 DTR / RTS 修改仍调用 `serial.signals`，不可变参数的提示使用共享前端组件 公共前端组件，后端接口不变。

「LF 补 CR」「CR 补 LF」是可同时勾选的前端接收显示偏好，默认均不选，按宿主提供的 `instanceConfigRef` 固定保存项标识保存在本地，不能用 COM、名称或运行 ID 替代；它们不属于下表的实例通信配置，不通过 `serial.configure` 发送。仅在显示时将选中的单独换行符补为 CRLF，已有 CRLF 不重复补全（含跨事件拆包）。`serial.read`、`serial.read_hex`、命令回显捕获、事件载荷、接收快照保存及字节计数始终保留原始内容。

终端普通 Backspace 的前端配置默认 BS，可选 DEL；仅物理按键映射为 `serial.write({text:"\b"|"\u007f",lineEnding:"none"})`，不修改后端接口的控制字符透传语义。退格编码与视图、接收格式、时间戳、自动滚动、换行补全一同按保存项记忆，详见[使用说明](usage.md)。

| 字段 | 类型 | 后端默认 / 约束 |
|---|---|---|
| `port` | string | 必填；Windows 接受 `COM6` 等设备名，Unix 接受 `/dev/...` 设备节点 |
| `baudRate` | integer | 必填且大于 0；配置界面默认 `115200` |
| `dataBits` | integer | 默认 `8`，范围 `5–8` |
| `stopBits` | integer | 默认 `1`，支持 `1/2` |
| `parity` | string | 默认 `none`，支持 `none/odd/even` |
| `dtr` | boolean | 默认 `true`，打开端口时设置 |
| `rts` | boolean | 默认 `true`，打开端口时设置 |
| `mode` | string | 默认 `stream`，支持 `stream/shell`；旧保存配置不猜测设备类型 |
| `prompt` | string 或 null | Shell 必填，1–256 UTF-8 字节，无控制字符；填写完整可见提示符，如 `root@board:~# `。stream 可为 null |

数据模式与终端 / UART 显示视图独立。在创建页或断开后的配置中显式选择；切换显示视图不修改数据模式。新建实例的模式和提示符进入宿主已有的初始配置保存流程，运行中 `serial.configure` 仍只修改当前实例，不更新宿主保存的初始配置。

每个实例的 `capabilities` 明确返回 `readLines / readSelectedLines / readDialoguePairs`。stream 仅支持第一项读取能力，后两项调用返回不支持错误；它仍可使用已有的审批文本 / HEX 写入工具，不把字节流强行解释为 Shell。MCP 工具目录按驱动共享，是支持能力的并集；Shell 专用工具描述及目标实例能力共同约束调用，不为此新增跨进程消息或宿主筛选协议。

每个驱动进程最多 32 个实例，同一端口不能分配给多个实例。实例创建不自动打开串口；切换实例或视图不关闭连接。重新连接生成新的 `connectionId`，消费者需要按连接重新建立游标。打开、关闭连接会清理该实例旧提案；销毁实例和驱动退出会释放连接并清理提案。

## 返回结果、审批与取消

命令结果包含 `operationId / total / sent / remaining / bytes / finished / cancelled / deviceCompletion`。stream 的 `deviceCompletion` 为 `unknown`，发送完成仅表示写入并 flush 成功。Shell 另含 `stdin / stdout / stderr / streamMode / exitCode / status / timedOut / error / truncated`，空流保留空串；串口无法拆分 stdout/stderr，故 `streamMode:"merged"`、`stderr:""`、`exitCode:null`。

对话记录同样保留 `stdin/stdout/stderr`，并包含 `operationId / commandIndex / status / boundary / deviceCompletion / observedInput / truncated`。提示符匹配只令该对话 `status:"completed"`、`deviceCompletion:"prompt_observed"`，不证明业务成功；超时标为 `timed_out`，取消标为 `cancelled`，断线或交付不确定标为 `unknown`。原始按键不会直接作为人工命令存储；只有设备在已确认提示符后的回显行会形成 `observedInput:true` 的人工对话。登录过程、无回显密码及未确认边界的文本不推断为命令。

stream 启用定时等待时，兼容结果 `output` 含 `text / cursor / promptMatched / truncated / gap`；内部采集使用游标，Agent 不需管理。设备异步日志可能混入结果。Shell 提示符按完整可见行匹配，支持跨包 UTF-8、ANSI 颜色及 OSC；若实际输出与提示符完全相同，仍可能误判。应配置尽量具体的完整提示符；目录、权限或提示符变化后，由用户更新配置，或在下一次调用显式指定 `waitForPrompt`。不注入 Shell 包装器、不探测密码、不伪造退出码。

MCP 完成结果的机器可读副本位于 `structuredContent`，文本块包含发送摘要和捕获回显；非命令读取同样提供可读文本。`_meta["coterm/operation"]` 保存操作收据，不再次复制完整结果。

串口实例工具注册 descriptor 的 `_meta["coterm/returnOnApproval"]:true`。进入人工审批后，调用返回 `structuredContent.status:"pending_approval"`、操作 ID 和提案 ID；收据仍使用运行时状态 `awaiting_approval`。用户批准后查询原操作，不重新提交写入。无需审批时继续等待正常完成。协议级等待期限使用 `tools/call.params._meta["coterm/waitTimeoutMs"]`，与业务参数 `waitTimeoutMs` 分开，详见 [MCP 接入](../../../docs/architecture/mcp-integration.md)。

| 取消情况 | 驱动 outcome | 行为 |
|---|---|---|
| 列表尚未开始 | `not_submitted` | 停止整个列表 |
| 列表已经开始 | `stopped` | 等当前写入确认后停止余下项；回显等待也可取消 |
| 无法确认是否停止 | `unknown` | 保留交付不确定性，禁止自动重放 |
| 单次写入或其他操作 | `unsupported` | 不支持取消 |

这些 outcome 是驱动取消响应，MCP 外层取消状态以宿主操作收据为准。断开、读写故障或写入超时会丢弃尚未发送的列表项，当前项可能部分送达；重新连接不恢复旧队列。

## 读取、事件与容量

`serial.read/read_hex` 使用连接内非负块序号游标，不是字节偏移；文本和 HEX 共用序号，每个消费者独立保存。游标过期时从仍保留的数据继续并报告 `gap:true`，超出末尾时报错；`more:true` 表示应继续分页。文本增量解码保留跨块 UTF-8 尾字节，非法编码用替换字符表示，HEX 精确保留原字节。

`serial.read_lines` 按 LF 分行，返回后端有界缓冲的尾部快照，重复调用可读到相同内容，不维护隐式共享游标。它不是 xterm 屏幕行，保留设备文本和控制序列；非法 UTF-8 用替换字符表示。`dropped` 表示缓冲淘汰过旧块，`truncated` 表示本次结果又因预算裁剪。`get_recent` 仅作内部兼容，不向 MCP 注册。MCP 不再注册 `read/read_hex/get_recent`，前端推送、缺口恢复和 HEX 导出仍使用内部回放接口。

命令、对话及选中读取都查询后端 JSON 快照，不依赖前端窗口存活。对话保留最近 32 项，每项 stdout 最多 16 KiB；输出溢出后继续识别提示符并标记截断。单次对话结构化结果预算为 12 KiB，可读预览不重复完整数据。选中快照最多 16 KiB，前端裁剪也保留截断标记；清空、断开及连接更换清除选中内容。断开保留该实例当前的有界输出和对话快照，重新配置、重新连接或销毁实例后不混用旧数据。数据只驻留内存，不新增明文落盘日志。

| 事件 | 主要 payload | 用途 |
|---|---|---|
| `serial.output` | `connectionId / text / bytes / cursor` | 新输入文本，不附 HEX 或 Base64 副本 |
| `serial.status` | `connected / connectionId`，关闭时带 `cursor` | 连接状态变化；完整状态用 status 查询 |
| `serial.command` | 命令进度字段及 `connectionId / txBytes` | 发送进度，不含命令内容 |

前端通过基座 `subscribeEvents()` 订阅，卸载时注销；MCP 目前不提供这些事件的订阅。事件队列有界，推送可丢失，回放缓冲是权威来源。发现游标不连续或 `runtime.gap` 后重新读取；HEX 视图以事件触发 `read_hex`。

| 边界 | 当前限制 |
|---|---|
| 每连接输出缓冲 | 256 KiB 原始数据及有界文本副本 |
| 单块 / 单次增量读取 | 最多 1 KiB 原始数据 |
| 回显与近期快照 | JSON 转义后文本预算仍为 6 KiB，文本和结构化两份保持有界；宿主输出记录现独立限 192 KiB，不改变本驱动预览限制；截断时标记 `truncated` |
| 每实例在途列表 | 最多 16 个，文本 / HEX 共用 FIFO；列表之间不交叉发送 |
| 写入 actor / 前端输入队列 | 64 槽 / 16 KiB；有在途列表时拒绝人工写入及信号修改 |
| 打开端口 / 每项驱动写入 | 阻塞线程池执行，等待上限分别为 5 秒 / 2 秒 |
| 读取与关闭 | 独立读取线程，读取轮询 20 ms；显式断开回收读取线程的等待上限为 2 秒 |

部分虚拟串口驱动不实现 flush（Windows 错误码 1/50）。仅在驱动明确不支持时，有界查询发送队列并确认排空后才计为发送成功；失败或超时不重发。结构化故障信息包含 IO 阶段、错误种类及 OS 错误码，不包含输入内容。

文本 / HEX 写入及命令列表均为敏感输入，普通账本只保留脱敏占位；秘密不应通过 Agent 参数提交。尚无硬件流控握手、自动重连、热插拔监听和文件收发。

接口实现见 [backend.rs](../src/backend.rs)、[command.rs](../src/command.rs)、[session.rs](../src/session.rs) 和 [payload.rs](../src/payload.rs)。调用示例与验收命令见 [使用说明](usage.md)。

桌面参数更新成功后调用前端 API `saveConfiguration(config)`，复用现有加密保存项；写盘失败保留旧保存值并阻止连接，重试不会发送设备命令。原生 configure 不承诺独立持久化。
