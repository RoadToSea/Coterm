# 基座与插件后端运行时通信

> **状态**：运行时通信设计稿，尚未冻结为 v1 线缆协议。
>
> 本文定义基座与插件后端之间的公共承载契约。它不定义 SSH、UART、MQTT、CAN 或其他协议的领域语义；插件作者只需要依赖本文描述的运行时 SDK，不需要了解基座内部实现。

供评审的 Rust/TypeScript 类型与待定项见[接口评审入口](interface-review.md)。本轮仅声明逻辑消息，未实现或冻结帧格式。

## 1. 设计目标

运行时通信需要同时承载四类事情：

```text
Lifecycle   插件进程和实例的启动、停止、健康状态
Operation   一次有明确结果的领域调用、草稿、提交、取消和状态
Interaction 插件向用户请求文本、秘密、确认或选择
Stream      持续的文本、二进制、结构化事件和订阅数据
```

它们使用同一条全双工异步连接，但保持不同的逻辑语义。基座只负责公共承载、关联、超时、取消、交互路由和操作记录；插件负责协议连接、领域参数、协议状态、缓存和持久化。

运行时必须满足：

- 同一个插件进程可以服务多个 `plugin_instance_id`，且所有消息都显式关联实例；
- 基座和插件都可以随时发送事件，不能把插件当作只能响应请求的 HTTP 服务；
- 请求等待超时不等于底层操作取消；
- 控制消息可以可靠传递，持续数据不会被强制塞进无限增长的控制队列；
- 插件重启、断线和结果不确定时，基座不能伪造成功或自动重放有副作用的调用。

## 2. 逻辑消息模型

### 2.1 基座发送给插件

```text
hello                 握手、版本和特性协商
create_instance       创建插件实例
destroy_instance      销毁插件实例
invoke                调用插件定义的领域操作
approve               批准待提交的 Agent 操作
reject                拒绝待提交的 Agent 操作
cancel                请求取消操作、交互或流
interaction_result    返回用户交互结果
stream_subscribe      请求投递某个插件数据流
stream_unsubscribe    停止某个界面的流投递
shutdown              请求插件有序退出
```

`approve` 和 `reject` 只表示基座对待提交操作的公共审批结果。基座不解析 CAN 字段、Shell 命令或 MQTT 参数；插件根据自己的草稿和领域规则完成实际提交。

### 2.2 插件发送给基座

```text
hello_result          返回握手结果和已启用特性
ready                 插件后端可以接受调用
instance_status       报告实例生命周期状态
operation_started     操作已被插件接受
operation_progress    可选的进度信息
operation_output      操作关联的脱敏输出或领域事件
operation_completed   操作成功完成并返回领域结果
operation_failed      操作失败、拒绝、取消或结果未知
stream_open           打开持续数据流
stream_data           投递数据块
stream_gap            报告序列缺口、丢弃或过期
stream_close          关闭数据流并给出原因
interaction_request   请求基座向用户收集输入
log                   诊断日志，不承载领域结果
```

插件只发送自己拥有的领域结果和事件。公共终态由基座按 `operation_id` 记录，插件不能通过日志伪造操作成功。

## 3. 关联标识和来源

不同类型的 ID 用于避免多实例和异步事件串线：

| 标识 | 作用 | 生命周期 |
|---|---|---|
| `plugin_id` | 插件类型 | 安装版本范围内稳定 |
| `plugin_instance_id` | 连接或工作单元 | 实例存活期间唯一 |
| `request_id` | 一次运行时请求 | 当前连接内唯一 |
| `operation_id` | 基座记录的一次逻辑操作 | 操作记录保留期内稳定 |
| `proposal_id` | Agent 草稿/提交审批关联 | 该次提议结束后失效 |
| `stream_id` | 持续数据流 | 流关闭后失效 |
| `interaction_id` | 一次用户交互 | 响应、取消或超时后失效 |

调用来源由基座在运行时上下文中标记：

```text
source = human | agent | system
```

插件不能从领域参数中声明或伪造来源。来源会影响基座是否拦截 Agent 提交，但不会改变插件对参数和协议权限的校验。

## 4. 物理传输建议

第一版建议使用插件进程的标准输入/输出建立一条全双工连接，并使用**明确长度的帧**承载消息。规范编码的单行 JSON 可以可靠承载转义文本；这里选择长度帧，是为了同时承载原始二进制和控制消息，并明确消息大小限制。SDK 仍须处理部分读写。

建议的物理层分为控制帧和数据帧，但对插件 SDK 暴露统一的异步消息 API：

```text
Frame = fixed header + payload

Control frame: JSON UTF-8 envelope
Data frame:    metadata envelope + raw bytes
```

控制帧只承载生命周期、调用、交互、状态和流控制信息。`stream_data` 的大块或原始字节使用数据帧，避免 base64 带来的体积和复制开销。结构化小数据可以直接放在控制帧中；二进制编码方式由握手特性声明。

帧头至少需要能确定：协议版本、帧类型、负载长度和校验/完整性策略。具体字节布局、最大帧长、端序和压缩方式在 v1 冻结时由 SDK 和 JSON Schema 一起发布。插件不得自行解析宿主私有字段。

插件的 `stdout` 仅用于运行时协议。诊断日志写入 `stderr` 或 `log` 消息；不得向 `stdout` 打印未封装的文本，否则基座应视为协议损坏并断开插件。

## 5. 握手和能力协商

插件启动后先发送或等待 `hello`，在握手完成前不接受领域调用。握手交换：

- 运行时接口主版本和兼容的次版本；
- 插件 ID、版本和平台信息；
- 支持的逻辑消息和可选特性；
- 最大控制帧、最大数据块和初始流窗口；
- 是否支持多实例、交互、取消、状态查询和二进制数据帧。

示例：

```json
{
  "type": "hello",
  "requestId": "hello-1",
  "interfaceMajor": 1,
  "interfaceMinor": 0,
  "features": [
    "instances",
    "interactions",
    "binary-streams",
    "flow-control",
    "operation-status"
  ],
  "limits": {
    "maxControlBytes": 262144,
    "maxDataChunkBytes": 1048576,
    "initialWindow": 64
  }
}
```

缺少必需特性时，基座拒绝启动插件并给出兼容性错误。未知的可选字段可以忽略；未知的必需消息或特性必须明确返回“不支持”，不能猜测执行。

## 6. 操作、草稿和提交

`invoke` 的 `operation` 和 `args` 完全由插件定义，基座只做路由和公共控制：

```json
{
  "type": "invoke",
  "requestId": "req-17",
  "operationId": "op-17",
  "proposalId": "proposal-4",
  "pluginInstanceId": "instance-1",
  "source": "agent",
  "effect": "commit",
  "operation": "can.send_frame",
  "waitTimeoutMs": 0,
  "args": {
    "id": 291,
    "data": "AQID"
  }
}
```

操作规则：

- `waitTimeoutMs > 0` 只限制调用方等待结果的时间；`waitTimeoutMs = 0` 表示提交或创建后立即返回；未提供时使用基座默认值；
- `effect` 至少区分 `read`、`draft` 和 `commit`。插件定义每个领域操作支持哪些效果；基座只根据声明决定是否拦截，不解析领域参数；
- 等待超时后，操作可以继续运行，基座将 `wait_expired` 记录为真；状态查询由基座/MCP Bridge 的公共 `get_operation` 提供，取消使用显式的 `cancel_operation`，两者都不要求插件再维护一份公共操作状态；
- Agent 的写操作在未启用 Bypass 时先形成 `proposal_id`，基座拦截提交并等待用户批准；批准后插件提交当前草稿，用户可以在插件界面中修改草稿；
- 人工直接提交的操作标记为 `source=human`，不经过 Agent 审批；Bypass 是用户主动承担风险的设置，只跳过基座的 Agent 审批，不绕过协议本身需要的认证或交互；
- `cancel` 是尽力请求。插件必须报告取消前未提交、已停止、已提交结果未知等明确结果；基座不自动重试有副作用的操作；
- `operation_completed`、`operation_failed` 只能出现一个终态。插件崩溃或连接断开时，已提交但未获结果的操作标为 `unknown`，不能报告成功。

推荐的 Agent 填写流程是：`effect=draft` 的调用进入插件并更新插件自己的草稿；`effect=commit` 的调用先由基座识别来源和 Bypass 设置。未批准时，基座保留 `proposal_id`，插件界面仍可展示和修改当前草稿；用户批准后，基座发送带 `proposal_id` 的 `approve`，插件提交此时的草稿。用户直接点击插件界面的发送按钮时，调用来源为 `human`，可以直接提交当前草稿。

## 7. 用户交互

插件通过 `interaction_request` 请求基座收集用户输入。交互必须带实例、操作和用途关联：

```json
{
  "type": "interaction_request",
  "interactionId": "interaction-8",
  "pluginInstanceId": "instance-1",
  "operationId": "op-17",
  "kind": "secret",
  "title": "SSH authentication",
  "prompt": "Password",
  "timeoutMs": 60000,
  "allowCancel": true
}
```

支持的公共类型为 `text`、`secret`、`confirmation` 和 `selection`。结果必须区分 `submitted`、`rejected`、`cancelled`、`timed_out`、`unavailable`。基座将秘密通过受控通道交给目标插件，普通事件、日志和 MCP 结果不得包含原始秘密。

交互等待不能阻塞插件消息循环。操作取消、实例销毁、插件退出或交互超时后，迟到的 `interaction_result` 必须丢弃，不能流向其他操作或实例。

## 8. 流和背压

流适用于 SSH 输出、UART 字节、MQTT 消息、CAN 报文和进度等持续数据。流消息至少带有 `stream_id`、`plugin_instance_id` 和单流递增的 `sequence`。

基座对每个订阅维护有界窗口。插件只能在窗口允许的范围内发送数据；窗口耗尽时必须暂停、丢弃并发送 `stream_gap`，或按插件声明的策略关闭流，不能无限占用内存。基座发送 `stream_subscribe` 时声明窗口和是否允许丢弃，插件返回实际策略。

```json
{
  "type": "stream_gap",
  "streamId": "stream-3",
  "pluginInstanceId": "instance-1",
  "fromSequence": 100,
  "toSequence": 118,
  "reason": "slow_consumer",
  "recoverable": true
}
```

界面卸载只停止该界面的投递，不自动关闭协议订阅或物理连接。需要完整历史的插件应先写入自己的存储，并通过快照、游标或领域 MCP 工具恢复；基座不承诺跨进程重启的恰好一次投递。

## 9. 断线、关闭和恢复

基座发送 `shutdown` 时，插件停止接受新实例和新操作，完成可安全完成的清理，然后发送最终状态并关闭连接。基座主动销毁实例时，插件结束关联交互和流，并释放协议资源。

异常断线时：

1. 基座把未结束的操作标为失败或结果未知，依据是否已收到插件的提交确认；
2. 迟到的消息不允许复用到新启动的进程或新实例；
3. 基座不会自动重放写操作；
4. 插件重新启动后重新握手，实例是否恢复由插件和基座的恢复策略决定；
5. 需要恢复的插件通过自己的持久化和领域查询确认设备状态，而不是依赖基座猜测。

## 10. SDK 对插件作者隐藏的内容

插件作者只实现异步处理器和事件订阅：

```text
runtime.onInvoke(operation, handler)
runtime.onCancel(handler)
runtime.createStream(metadata)
runtime.requestInteraction(request)
runtime.reportStatus(status)
```

SDK 负责帧编码、请求关联、窗口更新、心跳、进程关闭和结构化错误。插件不应直接拼接帧、不应依赖 `stdout` 的具体字节布局，也不应读取基座的私有文件或 IPC。

## 11. v1 冻结前的验证

当前运行时使用 JSON-RPC 行适配器。后端秘密交互采用协商后的 `_cotermInteraction` 控制应答，避免等待用户时阻塞控制请求；具体字段、期限和恢复方式见 [MCP 与后端秘密输入 MVP](mcp-stdio.md#插件作者的延迟交互契约)。主动事件和正式二进制帧仍未实现。

正式冻结前，至少需要用一个命令/请求型插件和一个持续订阅型插件验证：

- 多实例消息不会串线；
- 大块二进制不需要 base64 才能传输；
- 慢消费者不会导致基座或插件无限增长；
- `waitTimeoutMs=0`、取消、用户拒绝和结果未知可区分；
- Agent 审批、用户修改草稿和 Bypass 语义一致；
- 密码等秘密不会进入普通事件和日志；
- 插件崩溃后不会自动重放有副作用操作。

以下内容在冻结时一并发布：帧头二进制布局、JSON Schema、错误码、窗口更新消息、心跳和断线规则、SDK 类型及最小测试工具。本文在此之前属于架构设计，不是可直接依赖的实现规格。
