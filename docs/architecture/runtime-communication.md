# 运行时通信

**控制协议 v2.0**。固定现有 stdio JSON-RPC、实例调用、审批、单轮秘密交互和 JSON 通知；二进制流与后端扩展交互另行冻结。终端聚焦与内置驱动架构设计见[终端聚焦与内置驱动架构设计](../design/terminal-focus-and-in-tree-architecture.md)。类型见 [runtime](../../crates/coterm-proto/src/runtime)，唯一编解码入口为 [wire.rs](../../crates/coterm-proto/src/runtime/wire.rs)，可执行契约见 [control_contract.rs](../../crates/coterm-proto/tests/control_contract.rs)。

## 评审结论与兼容规则

现有分工适合 SSH、串口等不同协议：基座只管理实例、可信来源和公共交互，驱动保留领域 schema、结果与存储；MCP 与界面复用同一实例。控制与领域数据分离的方向保留。

单一 JSON 管道仍可能被大事件占用，不能作为高吞吐数据通道或安全沙箱。

- `interfaceMajor=2`、`interfaceMinor=0`；主版本不符拒绝。协商 minor 不超过双方支持值。
- 同一 major 只增加可忽略的可选字段或需协商的特性；改变字段、枚举语义、审批规则或物理编码必须升级 major。新枚举/方法不能未协商就发送。
- `enabledFeatures` 必须是双方能力交集，包含全部 required，不能重复或自行增项；未知 optional 忽略，未知 required 拒绝。
- manifest 的 `id/version` 必须与握手一致；`runtime.requires` 必须全部启用。驱动版本与运行时接口版本独立。
- `maxControlBytes` 协商为正数且不超过宿主报价和 1 MiB；首轮握手使用 1 MiB，成功后双向应用。`maxDataChunkBytes/initialWindow` 原样回显，当前无流控语义。
- `backend::DriverBackend` 是唯一 Rust 入口，旧 `transport::DriverBackend` 路径保留别名。`DriverEvent` 为内部类型，不能当作线缆通知；`stream_*` 明确返回不支持。

## 传输与生命周期

- 每行一个 UTF-8 JSON-RPC 2.0 对象，长度包含 LF（输入接受 CRLF）；不支持 batch，截断行关闭连接。请求 ID 为字符串，回复按 ID 关联；stdout 不混入日志。
- 宿主每进程最多一个控制请求在途，应答限时 10 秒；回复和通知共用串行输出，事件不能无限堆积。stdout 失败停止收件，shutdown 不等待遗留通知发送端。
- 先 `initialize`，成功前只允许握手和 `shutdown`；重复成功握手返回 conflict。失败握手可修正后重试，不允许在原进程重新协商已启用能力。
- 驱动只声明真正实现的能力；类型中存在某条消息不代表所有驱动支持它。
- Rust 驱动实现 `DriverBackend`，共享传输层负责读取、tick 和串行输出。
- Windows GUI 宿主启动驱动进程时使用 `CREATE_NO_WINDOW`，不分配或继承控制台；stdin/stdout 继续使用独立管道，stderr 保持诊断通道。macOS/Linux 的启动方式不变。控制台分配行为须在真实 Release Tauri GUI 中验收，仅从 `cargo test` 启动子进程不能覆盖 GUI 宿主场景。

| 方法/通知 | 参数 → 成功结果 | 特性 |
|---|---|---|
| `initialize` | `RuntimeHello` → `HelloResult`（操作与 MCP 目录） | 首条 |
| `create_instance` | `CreateInstance` → 成功 JSON，可带 `label` | instances |
| `destroy_instance/shutdown` | `{driverInstanceId}` / `null` → 成功 JSON | instances / 基础 |
| `invoke` | `InvokeRequest` → 领域结果、提案或保留信封 | backend-calls + operation-catalog |
| `approve/reject` | `ApproveProposal` / `{operationId,proposalId,reason?}` → 执行结果 / 成功 JSON | 同上 |
| `cancel` | `{type:"operation"\|"interaction",id}` → `{outcome:"not_submitted"\|"stopped"\|"unsupported"\|"unknown"}` | 基础 |
| `provide_secret` | `SecretDelivery` → 成功 JSON | secret-delivery |
| `interaction_result` | `InteractionResult` → 原操作结果 | deferred-interactions，依赖 secret-delivery |
| `coterm.notification` | `DriverNotification`，无回复 | async-operations / driver-events |

结构字段使用 camelCase，枚举标签使用 snake_case；领域 `args/result/payload` 由驱动 schema 定义。普通结果不得占用顶层 `_coterm*` 字段。成功回复恰有 `result`（可为 null），失败回复恰有 `error`。

长操作立即返回 `{"_cotermDeferred":"operation-id"}`，之后用 `operation_completed` 或 `operation_failed` 通知完成。通知不带 JSON-RPC `id`；领域事件使用 `type="event"`、`driverInstanceId`、`name` 和 `payload`。

## 错误约定

解析/信封/方法/参数错误分别为 JSON-RPC `-32700/-32600/-32601/-32602`；无法关联请求时 `id:null`。有效请求的错误不会结束进程，超限、截断、I/O 失败则关闭连接。

业务与运行时错误使用 `error.code=-32000`、`error.data={category,code,message,details}`，不从展示文字推断错误码。`category` 固定为 invalid_request / not_found / unsupported / conflict / interaction / transport / domain / internal；`code` 为稳定机器标识，`details` 只能放非秘密上下文。旧驱动无结构化 data 时降级为 `driver_rpc_<数值>`。

公共控制错误包括 `runtime_not_initialized`、`runtime_already_initialized`、`runtime_feature_not_negotiated`、`driver_handshake_incompatible`、`driver_control_too_large`。驱动领域码由驱动独立维护；`transport` 表示执行结果可能未知，不表示操作可重试，也不单凭分类终止健康进程。

## 操作与审批

`requestId` 关联控制应答；`operationId` 关联完整操作；`proposalId` 关联审批；实例和驱动 ID 划定授权域，不能互相替代。

1. 基座从调用入口确定来源：桌面为 Human，MCP 为 Agent；按注册目录解析 scope/effect 并校验参数。
2. 目标实例的独立审批对象接收已验证上下文；Agent commit 未被该实例的规则或 Bypass 放行时，调用驱动准备提案，不允许实际提交。驱动级提交不借用实例规则。
3. 用户修改、批准或拒绝草稿；批准时重新校验参数快照与目标，再派发。
4. 驱动返回执行事实；不符合可选输出 schema 的结果记为 unknown。

等待上限只结束调用方等待，不取消操作。取消必须显式请求：驱动确认未提交或已停止才记为 cancelled，无法确认时为 unknown。断开、超时或崩溃均不得触发写操作自动重放。

宿主普通和 transient 账本各有每实例 100 条的上限；非终态记录受保护，容量被活动操作占满时拒绝新操作。每条输入最多保留 16 KiB，输出独立保留最多 192 KiB（包含原生 MCP 文本和结构化结果），按脱敏后的实际 JSON 大小计费；超限以 `_coterm_truncated` 标记代替。输出预算扩大不会放宽输入预算、审批快照或秘密规则。账本仅保留本次启动，退出清除；敏感参数与批准快照必须脱敏。

敏感提案的完整草稿单独保存在宿主待审批内存中，普通账本的输入和提案输出都仅保留脱敏占位，MCP 查询不能取回命令或秘密。经授权的 Human 单条操作读取，以及桌面专用的驱动级审批读取，会在待审批期间提供当前草稿供用户检查、编辑；批准、拒绝、取消、实例销毁或驱动关闭后清除该草稿，不持久化。

## 秘密与延迟交互

后端可返回 `_cotermInteraction` 控制信封，保存有界待恢复上下文后释放 RPC。用户响应由基座通过 `interaction_result` 恢复原操作，不将交互信封当作领域结果。

- 原始秘密通过 `provide_secret` 交给目标后端，普通交互结果仅带句柄。
- 句柄绑定实例、用途，30 秒有效且一次性消费；不能进入普通账本、事件或 MCP。
- 交互最多 32 个，宿主限制等待期限；取消、过期、实例销毁或驱动停止后拒绝迟到结果。
- 当前支持一次秘密交互；多轮交互、MFA 和通用秘密兑换 SDK 待完善。
- 秘密与控制消息仍共用物理 stdio，仅逻辑隔离；同 WebView 前端也不是安全沙箱。

## 实例事件与流

当前驱动通过有界 JSON 事件推送，领域字节可使用 base64。宿主按实例路由，不解析 SSH 命令或串口字节。

桌面 `subscribeEvents` 安装订阅后返回；处理器完成后确认投递。宿主最多 32 个订阅，每订阅广播队列 64 项，30 秒未确认会关闭。慢消费者收到 `runtime.gap` 后按驱动游标回放；连接 ID 防止旧连接数据混入。

取消订阅或卸载界面不销毁实例。正式流将使用 stream/subscription ID、序号、窗口信用和明确溢出策略；二进制帧、心跳、流恢复与编码规则需另行验证和冻结，不能把现有类型当成已实现接口。

## 宿主待处理状态

审批与交互跨驱动存在，使用宿主级 `host_attention` 快照，不借用实例事件通道：

- 快照含单调递增的 `version`、全部 `approvals` 和 `interactions`；条目按稳定 ID 排序。
- 审批条目含操作/提案/目标标识与显示信息；交互条目含交互/实例/操作标识、类型、提示、期限和剩余时间。
- `host://attention-changed` 只发送版本号。快照和通知均不含秘密值或操作参数；审批参数另按授权域从操作记录读取。
- 先注册监听，再读快照；忽略旧版本，合并并发补取。读取失败保留上一份状态，不清空列表。
- 多个组件共享订阅；卸载释放监听。迟到补取及旧弹窗完成回调不能覆盖新交互。
- 决策、取消、过期、实例销毁和驱动停止后收敛；条目消失即关闭对应交互，其他待处理项不受影响。

实现见 [host-api.ts](../../apps/desktop/src/host-api.ts) 与 [HostInteractions.tsx](../../apps/desktop/src/components/host/HostInteractions.tsx)，回归见 [tests](../../tests)。

## 工作区与生命周期

每个运行实例拥有独立命令审批对象；规则按保存项 configRef 持久化，Bypass 仅该实例本次会话有效。不再按 operation 名称或驱动 ID 批量放行。规则契约、正反向模式、跨平台保存和旧数据迁移见 [命令审批规则](command-approval.md)。

Windows 首版将版本化工作区、初始配置及配置引用整体用当前用户 DPAPI 加密，原子替换 `workspace.v1.dpapi`；文档版本为 3，兼容读取版本 1、2；旧 `pluginId` 与 `coterm-plugin-*` 标识迁移到 `driverId` 与 `coterm-driver-*`，保留 configRef、分组、配置、旧 retained 标记和各项审批规则。版本 1 的已有全局规则逐项复制。UI 可读配置引用、标签和每项规则，不能读取连接配置。最多 128 项、明文编码上限 512 KiB，不保存 Bypass、操作账本或运行期秘密句柄；配置中出现宿主秘密句柄结构时拒绝保存。损坏/不兼容文件显示错误，须用户显式重置；非 Windows 暂保留会话内工作区并明确提示，不回退明文存盘。

启动只读取保存项；用户可逐项或整体恢复。恢复分配新的实例/会话 ID，只调用创建，不重放历史操作、不自动连接。保存项保留顺序、选中引用与侧栏折叠状态；销毁实例移除保存项，停止驱动保留恢复入口。驱动领域数据仍由驱动管理。

工作区分组属于基座元数据，不进入驱动协议或 MCP 工具。快照新增 `groups: [{id, name}]`，保存项包含 `workspaceId`，布局包含 `activeWorkspaceId`；旧 v1 文件缺少这些字段时自动归入默认工作区。最多 32 组，名称经去除首尾空白后为 1–64 个字符，不含控制字符且不允许同名（大小写不敏感）。桌面专用 `create_workspace_group`、`delete_workspace_group`、`move_workspace_entry` 与创建/销毁实例共用工作区变更锁；`create_instance`、`save_workspace_layout` 可显式传入可选 `workspaceId` / `activeWorkspaceId`。删除分组将保存项移到剩余第一组，至少保留一组；移动不调用任何驱动操作，也不改变实例、会话或配置引用。

创建实例时可单独传入可选名称，不混入驱动配置或运行时协议。名称去除首尾空白后最多 256 个 UTF-8 字节且不含控制字符；空值沿用驱动返回标签或基座默认名。最终名称在运行时全局去重，重名时追加序号；工作区保存最终标签，并在恢复时重新使用该标签。

恢复快照只保存最近一次实际创建或恢复实例的启动批次：上一批未恢复的项本次仍可查看和恢复，但一旦本次开始使用实例，后续落盘只包含本次实例；下一次启动不继续累积更早批次。仅打开应用而不创建/恢复实例，不清空上一批记录；删除本次最后一个实例后保存空批次，不使旧记录重新出现。停止或失败实例仍保留本批配置引用。文件写入失败时分组、成员关系、批次标志和原文件均保持原状态。旧文件未记录启动批次，升级时不猜测其历史来源；从新版第一次实际使用实例后按新规则收敛。



进程断开后移除运行态和操作目录，实例收敛为 `failed`；未派发操作取消，已派发/运行中的结果标为 `unknown`，并清理审批和交互。失败实例可删除或从保存配置重建；旧实例不能路由到新进程。退出停止事件订阅，并有界关闭、强制回收子进程。


## 内置分发与配置保存

桌面只发现 SSH、Serial、Local 三个固定目录。开发构建读取仓库 `target/bundled-drivers/{ssh,serial,local}`；发行构建读取 Tauri resource 目录下的 `drivers/{ssh,serial,local}`。`pnpm drivers:build` 为当前平台构建前端与 Worker 并暂存；生产构建使用 `--release`。不扫描应用数据目录，不提供安装、升级、移除离线包的桌面 API。

运行时字段统一使用 `driverId/driverInstanceId`、操作作用域为 `driver|instance`、事件能力为 `driver-events`；旧 v1 Worker 不兼容，握手直接拒绝。前端契约为 `@coterm/driver-sdk` v2.0，MCP 的 `coterm_ssh_command` 等终端工具名保持稳定，实例列表字段使用 driver 命名。

前端通过绑定当前实例的 `saveConfiguration(config)` 保存驱动已验证的完整配置。桌面 `save_instance_configuration({driverInstanceId,config})` 与生命周期变更共用工作区锁，仅替换对应保存项配置，保留 configRef、名称、分组及审批规则。拒绝失效实例、秘密句柄、超限配置与写盘失败；失败不覆盖原保存值。串口配置保存成功后才连接，恢复不自动连接。
