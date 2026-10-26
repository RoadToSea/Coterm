# 运行时通信

**基线：控制协议 v1.0（2026-10-26）**。固定现有 stdio JSON-RPC、实例调用、审批、单轮秘密交互和 JSON 通知；前端 SDK v1.0 另见[开发指南](plugin-development.md)；二进制流与后端扩展交互另行冻结。类型见 [runtime](../../crates/coterm-proto/src/runtime)，唯一编解码入口为 [wire.rs](../../crates/coterm-proto/src/runtime/wire.rs)，可执行契约见 [control_contract.rs](../../crates/coterm-proto/tests/control_contract.rs)。

## 评审结论与兼容规则

现有分工适合 SSH、串口等不同协议：基座只管理实例、可信来源和公共交互，插件保留领域 schema、结果与存储；MCP 与界面复用同一实例。控制与领域数据分离的方向保留。

本轮修正：两套同名后端入口合为一个；消除多处方法编码副本；保留结构化错误；落实握手顺序、能力交集、双向字节限制及输出失败后的退出；无法确认的异步受理记为 unknown。单一 JSON 管道仍可能被大事件占用，不能作为高吞吐数据通道或安全沙箱。

- `interfaceMajor=1`、`interfaceMinor=0`；主版本不符拒绝。协商 minor 不超过双方支持值。
- 同一 major 只增加可忽略的可选字段或需协商的特性；改变字段、枚举语义、审批规则或物理编码必须升级 major。新枚举/方法不能未协商就发送。
- `enabledFeatures` 必须是双方能力交集，包含全部 required，不能重复或自行增项；未知 optional 忽略，未知 required 拒绝。
- manifest 的 `id/version` 必须与握手一致；`runtime.requires` 必须全部启用。插件包版本与运行时接口版本独立。
- `maxControlBytes` 协商为正数且不超过宿主报价和 1 MiB；首轮握手使用 1 MiB，成功后双向应用。`maxDataChunkBytes/initialWindow` 原样回显，当前无流控语义。
- `backend::PluginBackend` 是唯一 Rust 入口，旧 `transport::PluginBackend` 路径保留别名。`PluginEvent` 为内部类型，不能当作线缆通知；`stream_*` 明确返回不支持。

## 传输与生命周期

- 每行一个 UTF-8 JSON-RPC 2.0 对象，长度包含 LF（输入接受 CRLF）；不支持 batch，截断行关闭连接。请求 ID 为字符串，回复按 ID 关联；stdout 不混入日志。
- 宿主每进程最多一个控制请求在途，应答限时 10 秒；回复和通知共用串行输出，事件不能无限堆积。stdout 失败停止收件，shutdown 不等待遗留通知发送端。
- 先 `initialize`，成功前只允许握手和 `shutdown`；重复成功握手返回 conflict。失败握手可修正后重试，不允许在原进程重新协商已启用能力。
- 插件只声明真正实现的能力；类型中存在某条消息不代表所有插件支持它。
- Rust 插件实现 `PluginBackend`，共享传输层负责读取、tick 和串行输出。

| 方法/通知 | 参数 → 成功结果 | 特性 |
|---|---|---|
| `initialize` | `RuntimeHello` → `HelloResult`（操作与 MCP 目录） | 首条 |
| `create_instance` | `CreateInstance` → 成功 JSON，可带 `label` | instances |
| `destroy_instance/shutdown` | `{pluginInstanceId}` / `null` → 成功 JSON | instances / 基础 |
| `invoke` | `InvokeRequest` → 领域结果、提案或保留信封 | backend-calls + operation-catalog |
| `approve/reject` | `ApproveProposal` / `{operationId,proposalId,reason?}` → 执行结果 / 成功 JSON | 同上 |
| `cancel` | `{type:"operation"\|"interaction",id}` → `{outcome:"not_submitted"\|"stopped"\|"unsupported"\|"unknown"}` | 基础 |
| `provide_secret` | `SecretDelivery` → 成功 JSON | secret-delivery |
| `interaction_result` | `InteractionResult` → 原操作结果 | deferred-interactions，依赖 secret-delivery |
| `coterm.notification` | `PluginNotification`，无回复 | async-operations / plugin-events |

结构字段使用 camelCase，枚举标签使用 snake_case；领域 `args/result/payload` 由插件 schema 定义。普通结果不得占用顶层 `_coterm*` 字段。成功回复恰有 `result`（可为 null），失败回复恰有 `error`。

长操作立即返回 `{"_cotermDeferred":"operation-id"}`，之后用 `operation_completed` 或 `operation_failed` 通知完成。通知不带 JSON-RPC `id`；领域事件使用 `type="event"`、`pluginInstanceId`、`name` 和 `payload`。

## 错误约定

解析/信封/方法/参数错误分别为 JSON-RPC `-32700/-32600/-32601/-32602`；无法关联请求时 `id:null`。有效请求的错误不会结束进程，超限、截断、I/O 失败则关闭连接。

业务与运行时错误使用 `error.code=-32000`、`error.data={category,code,message,details}`，不从展示文字推断错误码。`category` 固定为 invalid_request / not_found / unsupported / conflict / interaction / transport / domain / internal；`code` 为稳定机器标识，`details` 只能放非秘密上下文。旧插件无结构化 data 时降级为 `plugin_rpc_<数值>`。

公共控制错误包括 `runtime_not_initialized`、`runtime_already_initialized`、`runtime_feature_not_negotiated`、`plugin_handshake_incompatible`、`plugin_control_too_large`。插件领域码由插件独立维护；`transport` 表示执行结果可能未知，不表示操作可重试，也不单凭分类终止健康进程。

## 操作与审批

`requestId` 关联控制应答；`operationId` 关联完整操作；`proposalId` 关联审批；实例和插件 ID 划定授权域，不能互相替代。

1. 基座从调用入口确定来源：桌面为 Human，MCP 为 Agent；按注册目录解析 scope/effect 并校验参数。
2. Agent commit 且 Bypass 关闭时，调用插件准备提案，不允许实际提交。
3. 用户修改、批准或拒绝草稿；批准时重新校验参数快照与目标，再派发。
4. 插件返回执行事实；不符合可选输出 schema 的结果记为 unknown。

等待上限只结束调用方等待，不取消操作。取消必须显式请求：插件确认未提交或已停止才记为 cancelled，无法确认时为 unknown。断开、超时或崩溃均不得触发写操作自动重放。

宿主普通和 transient 账本各有每实例 100 条的上限；非终态记录受保护，容量被活动操作占满时拒绝新操作。账本仅保留本次启动，退出清除；敏感参数与批准快照必须脱敏。

## 秘密与延迟交互

后端可返回 `_cotermInteraction` 控制信封，保存有界待恢复上下文后释放 RPC。用户响应由基座通过 `interaction_result` 恢复原操作，不将交互信封当作领域结果。

- 原始秘密通过 `provide_secret` 交给目标后端，普通交互结果仅带句柄。
- 句柄绑定实例、用途，30 秒有效且一次性消费；不能进入普通账本、事件或 MCP。
- 交互最多 32 个，宿主限制等待期限；取消、过期、实例销毁或插件停止后拒绝迟到结果。
- 当前支持一次秘密交互；多轮交互、MFA 和通用秘密兑换 SDK 待完善。
- 秘密与控制消息仍共用物理 stdio，仅逻辑隔离；同 WebView 前端也不是安全沙箱。

## 实例事件与流

当前插件通过有界 JSON 事件推送，领域字节可使用 base64。宿主按实例路由，不解析 SSH 命令或串口字节。

桌面 `subscribeEvents` 安装订阅后返回；处理器完成后确认投递。宿主最多 32 个订阅，每订阅广播队列 64 项，30 秒未确认会关闭。慢消费者收到 `runtime.gap` 后按插件游标回放；连接 ID 防止旧连接数据混入。

取消订阅或卸载界面不销毁实例。正式流将使用 stream/subscription ID、序号、窗口信用和明确溢出策略；二进制帧、心跳、流恢复与编码规则需另行验证和冻结，不能把现有类型当成已实现接口。

## 宿主待处理状态

审批与交互跨插件存在，使用宿主级 `host_attention` 快照，不借用实例事件通道：

- 快照含单调递增的 `version`、全部 `approvals` 和 `interactions`；条目按稳定 ID 排序。
- 审批条目含操作/提案/目标标识与显示信息；交互条目含交互/实例/操作标识、类型、提示、期限和剩余时间。
- `host://attention-changed` 只发送版本号。快照和通知均不含秘密值或操作参数；审批参数另按授权域从操作记录读取。
- 先注册监听，再读快照；忽略旧版本，合并并发补取。读取失败保留上一份状态，不清空列表。
- 多个组件共享订阅；卸载释放监听。迟到补取及旧弹窗完成回调不能覆盖新交互。
- 交互组件补取的成功与失败都核对出发时的生命周期和观察版本。较新空快照使旧补取失效；订阅错误也推进生命周期，旧结果不能覆盖提示或打开队列中的下一项。当前补取失败仍显示可恢复错误，新快照可清除提示。
- 决策、取消、过期、实例销毁和插件停止后收敛；条目消失即关闭对应交互，其他待处理项不受影响。

实现见 [host-api.ts](../../apps/desktop/src/host-api.ts) 与 [HostInteractions.tsx](../../apps/desktop/src/components/host/HostInteractions.tsx)，回归见 [tests](../../tests)。

## 工作区与生命周期

Windows 首版将版本化工作区、初始配置及配置引用整体用当前用户 DPAPI 加密，原子替换 `workspace.v1.dpapi`；UI 只读配置引用和标签。最多 128 项、明文编码上限 512 KiB，不保存 Bypass、操作账本或运行期秘密句柄；配置中出现宿主秘密句柄结构时拒绝保存。损坏/不兼容文件显示错误，须用户显式重置；非 Windows 暂保留会话内工作区并明确提示，不回退明文存盘。

启动只读取保存项；用户可逐项或整体恢复。恢复分配新的实例/会话 ID，只调用创建，不重放历史操作、不自动连接。保存项保留顺序、选中引用与侧栏折叠状态；销毁实例移除保存项，停止插件保留恢复入口。插件领域数据仍由插件管理。

进程断开后移除运行态和操作目录，实例收敛为 `failed`；未派发操作取消，已派发/运行中的结果标为 `unknown`，并清理审批和交互。失败实例可删除或从保存配置重建；旧实例不能路由到新进程。退出停止事件订阅，并有界关闭、强制回收子进程。
