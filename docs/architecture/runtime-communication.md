# 运行时通信

当前采用 stdio JSON-RPC 行适配器；正式二进制帧和通用流接口尚未接通，公开契约未冻结。类型见 [runtime](../../crates/coterm-proto/src/runtime)，实际编码见 [transport.rs](../../crates/coterm-proto/src/runtime/transport.rs) 与 [plugin_process.rs](../../crates/coterm-core/src/plugin_process.rs)。

## 传输与生命周期

- 每行一个 JSON-RPC 2.0 消息，请求 ID 为字符串，回复按 ID 关联；stdout 不混入日志。
- 控制行上限 1 MiB，单次控制应答限时 10 秒；持续数据不能塞进无限大的控制消息。
- 先 `initialize` 协商身份、接口版本、能力、限制和操作目录，再接受领域调用。
- 插件只声明真正实现的能力；类型中存在某条消息不代表所有插件支持它。
- Rust 插件实现 `PluginBackend`，共享传输层负责读取、tick 和串行输出。

| 方法/通知 | 用途 |
|---|---|
| `initialize` | 握手与目录注册 |
| `create_instance/destroy_instance/shutdown` | 实例与进程生命周期 |
| `invoke/approve/reject` | 调用、提案批准/拒绝 |
| `cancel` | 取消操作或交互 |
| `provide_secret/interaction_result` | 私有秘密投递与交互恢复 |
| `coterm.notification` | 操作完成/失败及实例事件 |

长操作立即返回 `{"_cotermDeferred":"operation-id"}`，之后用 `operation_completed` 或 `operation_failed` 通知完成。通知不带 JSON-RPC `id`；领域事件使用 `type="event"`、`pluginInstanceId`、`name` 和 `payload`。

## 操作与审批

`requestId` 关联控制应答；`operationId` 关联完整操作；`proposalId` 关联审批；实例和插件 ID 划定授权域，不能互相替代。

1. 基座从调用入口确定来源：桌面为 Human，MCP 为 Agent；按注册目录解析 scope/effect 并校验参数。
2. Agent commit 且 Bypass 关闭时，调用插件准备提案，不允许实际提交。
3. 用户修改、批准或拒绝草稿；批准时重新校验参数快照与目标，再派发。
4. 插件返回执行事实；不符合可选输出 schema 的结果记为 unknown。

等待上限只结束调用方等待，不取消操作。取消必须显式请求：插件确认未提交或已停止才记为 cancelled，无法确认时为 unknown。断开、超时或崩溃均不得触发写操作自动重放。

宿主普通和 transient 账本各有每实例 100 条的上限；非终态记录受保护，容量被活动操作占满时拒绝新操作。账本在内存中，敏感参数与批准快照必须脱敏。

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
- 决策、取消、过期、实例销毁和插件停止后收敛；条目消失即关闭对应交互，其他待处理项不受影响。

实现见 [host-api.ts](../../apps/desktop/src/host-api.ts) 与 [HostInteractions.tsx](../../apps/desktop/src/components/host/HostInteractions.tsx)，回归见 [tests](../../tests)。
