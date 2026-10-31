# Serial 接口设计

## 设计边界与调用入口

插件负责串口连接、文本和原始字节收发、输出缓存及实例内的发送队列。它不解析设备协议、不组 Modbus 帧、不计算 CRC、不实现固件下载握手，也不推断远端命令退出码。`waitForPrompt` 仅匹配字面子串。

| 调用方 | 入口与路由 | 参数与结果约定 |
|---|---|---|
| 插件前端 | 基座 `api.invoke(operation, args)`；宿主绑定当前实例 | 业务参数不携带实例名称；返回通用操作收据，业务结果在 `output` |
| MCP 客户端 | `coterm-plugin-serial__serial.*` | 除 `plugin_info` 外必填公共实例名称 `name`；Bridge 解析实例并调用内部 `mcp.serial.*` 操作 |
| 插件进程 | 宿主通用生命周期、操作和通知协议 | `initialize/create_instance/destroy_instance/invoke/approve/reject/cancel/shutdown` 沿用基座协议，不作为 Serial MCP 工具开放 |

`name` 必须取自 `coterm.list_instances`，不能传内部 UUID。Bridge 运行时兼容 `instance`、`instanceName`、`instance_name`，一次调用只能提供一种名称字段；公开 schema 的规范字段仍是 `name`。参数采用严格 schema，未知字段会被拒绝。

## 操作总表

共 13 个业务操作，其中 10 个向 MCP 开放。下表省略 MCP 实例工具公共参数 `name`；`?` 表示可选。MCP 实际工具名需在表中操作名前加 `coterm-plugin-serial__`。

| 操作 | MCP | effect | 业务参数 | 业务结果 / 用途 |
|---|:---:|---|---|---|
| `serial.plugin_info` | 是 | read | 无 | `pluginId / instanceCount`；插件级操作，不绑定实例 |
| `serial.status` | 是 | read | 无 | `config / connected / connectionId / signals / rxBytes / txBytes / commandQueue` |
| `serial.probe` | 是 | read | 无 | status 结果及 `ports`；端口项含 `portName / portType`，USB 类型可带 VID/PID、厂商、产品与序列号 |
| `serial.connect` | 是 | commit | 无 | 按实例配置打开端口，返回 `connected:true`；已连接时拒绝重复打开 |
| `serial.disconnect` | 是 | commit | 无 | 关闭端口并丢弃剩余队列，返回 `connected:false` |
| `serial.read` | 是 | read | `cursor?: integer`，默认 `0` | 文本分页：`text / bytes / cursor / gap / closed / more / connectionId` |
| `serial.get_recent` | 是 | read | `lines?: integer`，默认 `50`，范围 `1–200` | 最近文本行快照：`text / lines / cursor / gap / closed / truncated / connectionId` |
| `serial.read_hex` | 是 | read | `cursor?: integer`，默认 `0` | 原始字节的 HEX 分页，字段同 read，以 `hex` 替代 `text` |
| `serial.command` | 是 | commit | `commands: string[]`，及下表可选参数 | 按序发送文本列表，可捕获回显；返回发送进度和可选 `output` |
| `serial.command_hex` | 是 | commit | `commands: string[]`、`intervalMs?` | 按序发送 HEX 列表；不补换行，不接受文本行结束符或等待回显参数 |
| `serial.write` | 否 | commit | `text: string`、`lineEnding?`，默认 `none` | 人工单次文本写入，返回 `submitted / bytes` |
| `serial.write_hex` | 否 | commit | `hex: string` | 人工单次 HEX 写入，返回 `submitted / bytes` |
| `serial.signals` | 否 | commit | `dtr?: boolean`、`rts?: boolean`，至少一个 | 仅修改给定信号，返回修改后的 `dtr / rts` |

`read` 操作不触发写入审批。Agent 来源的 `commit` 操作默认经过宿主审批；用户主动开启 Bypass 或明确放行后可跳过提交审批，协议自身的认证与人工交互仍保留。`write/write_hex/signals` 未注册 MCP descriptor，人工界面通过基座调用。

## 命令列表参数与编码

| 参数 | 类型 | 必填 | 默认值 | 规则 |
|---|---|:---:|---|---|
| `commands` | string[] | 是 | — | 1–64 项；文本列表按 UTF-8 编码，HEX 列表解码为原字节 |
| `lineEnding` | string | 否 | `cr` | 仅文本列表；支持 `none/cr/lf/crlf`，追加到每一项末尾 |
| `intervalMs` | integer | 否 | `100` | 文本和 HEX 列表均支持，范围 `0–1000 ms`，只在两项之间等待 |
| `waitForPrompt` | string | 否 | 不启用 | 仅文本列表；非空、区分大小写的字面子串，不是正则 |
| `waitTimeoutMs` | integer | 否 | 指定提示符但未指定期限时为 `5000` | 仅文本列表；范围 `0–300000 ms`，单独指定表示定时收集输出 |

未提供两个等待参数时，命令结果仅确认发送。每项实际载荷为 1–4096 字节，包含追加的行结束符；整批最多 16 KiB。文本空字符串在追加行结束符后非空时有效。整批先校验，任何一项不合法时均不发送。

文本保留 Ctrl+C、Tab、ESC、换行、DEL、NUL 等控制字符；原样输入必须显式设置 `lineEnding:"none"`。人工 `serial.write` 默认不追加结束符，与列表默认 CR 不同。

HEX 接受偶数位十六进制字符，可包含 ASCII 空白和一个开头 `0x`，不隐式追加任何字节。文本与 HEX 参数不能混用，不支持 Base64。人工单次写入总量为 1–16384 字节。

## 实例配置与生命周期

配置在创建实例时提供；`serial.connect` 本身不接收端口参数。

| 字段 | 类型 | 后端默认 / 约束 |
|---|---|---|
| `port` | string | 必填；Windows 接受 `COM6` 等设备名，Unix 接受 `/dev/...` 设备节点 |
| `baudRate` | integer | 必填且大于 0；配置界面默认 `115200` |
| `dataBits` | integer | 默认 `8`，范围 `5–8` |
| `stopBits` | integer | 默认 `1`，支持 `1/2` |
| `parity` | string | 默认 `none`，支持 `none/odd/even` |
| `dtr` | boolean | 默认 `true`，打开端口时设置 |
| `rts` | boolean | 默认 `true`，打开端口时设置 |

每个插件进程最多 32 个实例，同一端口不能分配给多个实例。实例创建不自动打开串口；切换实例或视图不关闭连接。重新连接生成新的 `connectionId`，消费者需要按连接重新建立游标。打开、关闭连接会清理该实例旧提案；销毁实例和插件退出会释放连接并清理提案。

## 返回结果、审批与取消

命令完成结果包含 `operationId / total / sent / remaining / bytes / finished / cancelled / deviceCompletion`。其中 `deviceCompletion` 为 `unknown`：发送完成仅表示所有字节写入并 flush 成功，不代表设备业务执行成功，不推断退出码，也不根据设备输出自动停止后续项。

启用等待时，`output` 包含 `text / cursor / promptMatched / truncated / gap`。收集从本批发送前的缓冲游标开始，发送后进行匹配与有界读取；这段数据可能包含设备异步输出，不是经过协议识别的独占命令响应。超时不代表设备失败。

MCP 完成结果的机器可读副本位于 `structuredContent`，文本块包含发送摘要和捕获回显；非命令读取同样提供可读文本。`_meta["coterm/operation"]` 保存操作收据，不再次复制完整结果。

串口实例工具注册 descriptor 的 `_meta["coterm/returnOnApproval"]:true`。进入人工审批后，调用返回 `structuredContent.status:"pending_approval"`、操作 ID 和提案 ID；收据仍使用运行时状态 `awaiting_approval`。用户批准后查询原操作，不重新提交写入。无需审批时继续等待正常完成。协议级等待期限使用 `tools/call.params._meta["coterm/waitTimeoutMs"]`，与业务参数 `waitTimeoutMs` 分开，详见 [MCP 接入](../../../docs/architecture/mcp-integration.md)。

| 取消情况 | 插件 outcome | 行为 |
|---|---|---|
| 列表尚未开始 | `not_submitted` | 停止整个列表 |
| 列表已经开始 | `stopped` | 等当前写入确认后停止余下项；回显等待也可取消 |
| 无法确认是否停止 | `unknown` | 保留交付不确定性，禁止自动重放 |
| 单次写入或其他操作 | `unsupported` | 不支持取消 |

这些 outcome 是插件取消响应，MCP 外层取消状态以宿主操作收据为准。断开、读写故障或写入超时会丢弃尚未发送的列表项，当前项可能部分送达；重新连接不恢复旧队列。

## 读取、事件与容量

`serial.read/read_hex` 使用连接内非负块序号游标，不是字节偏移；文本和 HEX 共用序号，每个消费者独立保存。游标过期时从仍保留的数据继续并报告 `gap:true`，超出末尾时报错；`more:true` 表示应继续分页。文本增量解码保留跨块 UTF-8 尾字节，非法编码用替换字符表示，HEX 精确保留原字节。

`serial.get_recent` 按 LF 分行，返回尾部快照，重复调用可读到相同内容；`gap` 表示缓冲曾淘汰早期块，`truncated` 表示选中的尾部文本又因结果预算被裁剪。它不维护隐式共享游标。

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
| 回显与近期快照 | JSON 转义后文本预算 6 KiB，保留文本和结构化两份后满足宿主 16 KiB 结果上限；截断时标记 `truncated` |
| 每实例在途列表 | 最多 16 个，文本 / HEX 共用 FIFO；列表之间不交叉发送 |
| 写入 actor / 前端输入队列 | 64 槽 / 16 KiB；有在途列表时拒绝人工写入及信号修改 |
| 打开端口 / 每项驱动写入 | 阻塞线程池执行，等待上限分别为 5 秒 / 2 秒 |
| 读取与关闭 | 独立读取线程，读取轮询 20 ms；显式断开回收读取线程的等待上限为 2 秒 |

部分虚拟串口驱动不实现 flush（Windows 错误码 1/50）。仅在驱动明确不支持时，有界查询发送队列并确认排空后才计为发送成功；失败或超时不重发。结构化故障信息包含 IO 阶段、错误种类及 OS 错误码，不包含输入内容。

文本 / HEX 写入及命令列表均为敏感输入，普通账本只保留脱敏占位；秘密不应通过 Agent 参数提交。尚无硬件流控握手、自动重连、热插拔监听和文件收发。

接口实现见 [backend.rs](../src/backend.rs)、[command.rs](../src/command.rs)、[session.rs](../src/session.rs) 和 [payload.rs](../src/payload.rs)。调用示例与验收命令见 [使用说明](usage.md)。
