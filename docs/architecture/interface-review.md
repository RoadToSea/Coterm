# 接口评审入口

接口仍是设计稿，**尚未冻结 v1**。当前已实现 core 操作历史账本、同步控制请求范围内的插件包加载/握手/进程/实例生命周期、桌面插件/实例管理、基于握手目录的操作路由、异步等待和人工审批，以及可信单文件插件 UI 挂载和人工秘密输入。SSH MVP 与接口扩展见 [SSH 插件](ssh-plugin.md)。固定地址的 MCP Streamable HTTP、实例级原生 Tools 和后端公共密码交互已接入 MVP，见 [运行方式与边界](mcp-http.md)；正式持续数据流和完整双向 MCP provider 尚未接入。

## 文件与职责

| 文件 | 评审内容 |
|---|---|
| [runtime/mod.rs](../../crates/coterm-proto/src/runtime/mod.rs) | 共享 ID、异步返回类型、公共错误 |
| [manifest.rs](../../crates/coterm-proto/src/runtime/manifest.rs) | 包入口、无 UI、多实例、运行时版本要求 |
| [lifecycle.rs](../../crates/coterm-proto/src/runtime/lifecycle.rs) | 握手、特性协商、实例生命周期 |
| [operation.rs](../../crates/coterm-proto/src/runtime/operation.rs) | 操作目录、来源、审批、状态与最近操作记录 |
| [interaction.rs](../../crates/coterm-proto/src/runtime/interaction.rs) | 文本、密码引用、确认、选择及退出原因 |
| [stream.rs](../../crates/coterm-proto/src/runtime/stream.rs) | 二进制逻辑载荷、每消费者订阅、窗口和缺口 |
| [message.rs](../../crates/coterm-proto/src/runtime/message.rs) | 双向控制消息和执行事件 |
| [backend.rs](../../crates/coterm-proto/src/runtime/backend.rs) | 插件作者实现的入口与 SDK 提供的宿主接口 |
| [plugin_runtime.rs](../../crates/coterm-core/src/plugin_runtime.rs) | 基座生命周期、操作、公共交互和流路由 |
| [bridge.rs](../../crates/coterm-mcp/src/bridge.rs) | 目标架构：原生 MCP 目录、双向消息和单入口路由 |
| [server.rs](../../crates/coterm-mcp/src/server.rs) | 可运行的 Tools MCP 服务与原生工具路由 |
| [http.rs](../../crates/coterm-mcp/src/http.rs) | 固定本机 URL、Streamable HTTP 会话和目录通知 |
| [interaction.rs](../../crates/coterm-proto/src/runtime/interaction.rs) 中的 DeferredInteraction | MVP 后端秘密交互的控制应答 |
| [plugin-api.ts](../../apps/desktop/src/plugin-api.ts) | 绑定实例的插件前端 SDK 与 mount/unmount |
| [host-api.ts](../../apps/desktop/src/host-api.ts) | 桌面壳管理实例、Bypass 和前端挂载的接口 |

操作历史实现位于 `crates/coterm-core/src/operation_history.rs`：每插件实例保留最近 N 条，支持状态转换、查询、等待超时标记、常见凭据字段脱敏和记录值大小上限。当前内存实现不跨进程重启持久化。

## MVP 实施顺序

1. **公共操作账本**（已完成）：有界记录、状态迁移、查询、等待超时标记。
2. **插件进程控制闭环（已完成 MVP）**：manifest 读取与基本校验、包内后端/前端路径校验、子进程启动/停止、握手和基础特性检查、多实例创建/销毁与列表、请求关联、10 秒超时及有界 JSON-RPC 行。单次控制请求仍串行，已支持插件主动报告操作完成和实例事件；二进制流、正式 MCP 复用及进程崩溃后的状态恢复尚未实现。JSON-RPC 行协议只是 MVP 适配器，不是冻结的正式线缆协议。
3. **操作路由（MVP 已完成）**：握手注册并编译插件操作目录；基座按 descriptor 的 `scope/effect` 和注册的 JSON Schema 校验并路由，人工调用来源由 Tauri 固定，Agent commit 默认进入 PrepareProposal，用户可用草稿快照批准或拒绝，Bypass 由用户设置并跳过基座审批；批准时再次校验用户修改后的快照。可选输出 schema 也会校验，格式不符时标记 `unknown`，避免把可能已产生副作用的调用误报成成功。操作账本保存脱敏摘要；等待、transient 记录和推送已接入。接受态、待审批和待交互操作可以取消；已派发/运行中的通用取消未接入，MCP `coterm.cancel_operation` 不承诺中断已提交的协议命令。插件级操作目前只允许宿主内部 `System` 来源调用，桌面/MCP 不开放；接入前需要定义独立于领域目标的插件级授权会话，不能借用某个实例会话扩大权限。
4. **桌面闭环（MVP 已完成）**：Tauri HostAPI 已接入应用数据目录中的插件发现、启停、实例管理、操作目录、操作调用、历史查询和审批命令；开发模式发现工作区插件构建包。插件前端 mount/unmount、可选 configure 及宿主秘密输入适配器已接入；操作参数表单按 JSON Schema 提供常用标量、布尔值和 enum 输入，并保留高级 JSON 编辑入口。审批草稿仍允许编辑 JSON。其他后端交互类型尚未接入。终端插件的命令块由终端插件定义，基座只记通用操作。
5. **MCP 闭环（已完成 Tools MVP）**：一个固定地址的 Streamable HTTP 入口、插件显式原生工具发现和路由、目录变化通知，固定 Agent 来源；原请求默认等待审批/秘密交互和执行后返回最终结果，仅立即返回或等待超时时返回 pending 凭据供后续查询。Resources、Prompts、订阅与通用取消仍未实现。

每一步完成后运行相应 crate 测试和桌面构建，再继续下一步。跨进程帧编码、二进制持续流及正式 MCP 复用方式仍按评审决定冻结；当前通过有界 JSON 通知推送 SSH 输出，不代表正式二进制流或完整 StreamRouter 已实现。

建议从 `plugin-api.ts`、`operation.rs`、`backend.rs` 开始，再看运行时消息与 MCP。

## SSH 实机验收（2026-10-21）

以下记录是改动前的实机验收。复用用户在桌面中已经连接的 SSH 实例，通过外部 stdio MCP 测试，未重启应用或重建 SSH 连接。

- 已验证：MCP 初始化、工具发现、实例定位、连接状态和游标读取；Agent 命令先返回 `awaiting_approval`，未批准时没有产生对应测试输出；待审批操作可取消，伪造 Human 来源及调用未公开审批工具被拒绝。
- 已验证人工修改：用户在桌面将命令改为 `echo COTERM_TEST_EDITED_20261021_OK` 后批准，MCP 读取到独立一行修改后的标记，SSH 仍连接，游标没有报告缺口。
- **当时的最终状态查询未通过验收**：批准后查询原操作返回 `operation_not_found`。当时每实例只保留 100 条记录，终态记录与高频输出/状态轮询共用容量，结果可能在 Agent 查询前被淘汰；后续修正见下节。
- **审批显示仍需排查**：用户曾反馈不弹窗及弹窗消失，重新提交后可以显示并完成审批，原始问题尚未稳定复现、根因未确认。新待审批记录经过 125 次状态查询仍可查到，不能将现象直接归因于待审批记录被淘汰。前端已增加审批查询失败提示，避免静默失败；提示不等于修复原始问题。
- 本次未实机覆盖新连接密码交互、Bypass、多实例、断线重连及取消已执行命令。此前自动化覆盖不能替代这些实机验收。

### 后续修正：等待结果与推送（已实现）

- 基座分配操作 ID 后异步派发，默认等待 60 秒，支持 `waitTimeoutMs=0` 和等待超时收据；MCP 同一连接可并发等待，原请求跨审批和执行直接返回结果。
- SSH 插件自行识别 POSIX Shell 完成边界，返回有界输出与退出码；断线或边界无法确认时报告 `unknown`。基座仅消费通用完成通知。
- SSH 页面使用实例事件推送及缺口回放，移除空闲输出/状态轮询和插件内重复审批面板。后台读取、按键与 resize 使用独立的有界 transient 记录区；普通最近 N 步不再被它们挤占。
- 自动化验证包括同连接等待与并发查询、人工修改后的原请求结果、非零退出码、125 次读取后记录保留、立即返回与取消、断线后 unknown；原审批弹窗偶发问题仍未确认根因。实现边界见 [SSH 插件](ssh-plugin.md) 和 [MCP 等待语义](mcp-stdio.md#等待与立即返回)。
- 验证通过：`cargo check --workspace`、25 项工作区测试、Clippy（警告视为错误）、桌面与 SSH 前端构建；浏览器使用模拟 Tauri IPC 验证推送确认、空闲不轮询、缺口恢复和卸载取消，不能替代真实 WebView 验收。开发插件包已更新。

### 更新后的远程实机复验（2026-10-21）

复用用户重新连接的 SSH 实例，通过外部 stdio MCP 验证，期间未重启应用、未重建 SSH 连接。

- **等待与人工修改通过**：命令请求设置 300000ms 等待，用户将草稿改为 `echo COTERM_WAIT_EDITED_20261021_OK` 后批准；同一个原始 MCP 请求直接返回 `completed`、`waitExpired=false`、`exitCode=0`、`isError=false`。输出只有修改后的标记，未出现原始标记，且未截断；等待期间未轮询操作结果。
- **默认等待与失败结果通过**：第二条命令输出测试标记后执行 `false`，未指定等待参数；用户批准后，原请求直接返回 `exitCode=1`、`isError=true`、`waitExpired=false`，保留命令输出。操作记录为 `completed` 表示执行已结束，业务成功与否由插件的退出码和原生 MCP 结果表示。
- **记录可查询**：首条命令完成后，在后续命令等待审批期间仍可按 ID 查询到 `completed`；此前 125 次读取的容量隔离验证来自自动化，本次未重复高频查询。
- 本轮两次人工审批均完成，但不能据此确认此前偶发弹窗问题已经修复。真实 WebView 的推送、慢消费者与缺口恢复，以及密码交互、Bypass、多实例和断线重连仍未完成本轮实机验收。

## 固定 MCP 连接配置（已实现）

用户只配置 `http://127.0.0.1:23517/mcp`，无需填写 Token、环境变量或转接程序路径。基座提供标准 Streamable HTTP，应用重启后地址不变；MCP 面板显示地址并支持复制。

- 仅监听本机回环，校验 Host 和请求中存在的 Origin，不开放通配 CORS；该模式不区分本机进程身份，不提供远程访问或多用户隔离。
- 复用已有实例、工具路由、Agent 来源、人工审批、Bypass 和原请求等待语义，不改变插件接口。
- 会话自动协商，旧会话失效返回 `404`；客户端重新初始化，不自动重放命令或恢复旧实例 ID。端口冲突显示错误，不切换随机端口。
- 会话、在途请求、消息和事件流均有上限，应用关闭时结束 HTTP 等待和 SSE。详细约束与自动化覆盖见 [固定 MCP 连接](mcp-http.md)。

旧 stdio 转接器保留用于开发回归，桌面不再发布旧地址和 Token。此前远程实机记录对应 stdio 传输，不能代替新 HTTP 入口的远程 SSH 与真实客户端验收。

## 本轮具体化的约束

- `source`、`operation_id` 由基座创建，`effect` 从插件注册目录读取。前端调用只传操作名、参数和等待选项，不自行指定来源或风险等级。
- 插件握手时基座编译并校验每个操作及原生 MCP Tool 的 JSON Schema；提交和批准均按注册 schema 校验完整参数。MCP Tool 的 schema 可以包含 `pluginInstanceId`，普通实例操作的 schema 不应依赖宿主隐式补字段。
- `waitTimeoutMs=0` 返回 `OperationReceipt`，包含操作 ID、可选提议 ID 和状态。等待超时只是 `waitExpired=true`，查询和取消由基座统一提供。
- 一个工具同时“填写+发送”时，基座可派发 `PrepareProposal`，插件只填写草稿、不发送。用户修改后批准，`ApproveProposal.args` 携带当时的草稿快照，防止审批后再读取已变化的草稿。
- 插件只报告执行事实，操作记录由基座维护。每个实例最近 N 步和每条记录大小都受配置限制；原始流、完整历史由插件保存。插件级操作使用独立的插件记录分组。
- 秘密通过 `SecretHandle` 关联实例、用途和有效期；后端通过私有兑换接口取用。`SecretMaterial` 不声明序列化或调试输出能力，但本轮不实现内存擦除。
- 二进制流类型不派生 JSON 序列化，避免误用 JSON 字节数组代替二进制帧。窗口同时限制块数和字节数，每个消费者有独立订阅 ID。
- MCP 使用原生 Request/Response/Notification 表达，不把 Resources、Prompts、订阅通知改成统一领域调用。
- Rust 可序列化 DTO 字段使用 camelCase；TypeScript 中可空字段使用 `null` 对齐 Rust `Option`。流序号在前端使用十进制字符串，避免 JS 整数精度丢失。

## 需要评审的选择

1. **组合工具的准备阶段**：插件是否接受 `PrepareProposal`；无 UI 插件是否使用基座公共确认展示草稿 JSON。此阶段必须禁止协议写入。
2. **审批快照**：用插件定义的完整 `args` 作为用户批准的发送内容是否足够；需要版本号/幂等键的插件可在领域参数中声明。
3. **MCP 等待与立即返回**：终态保持原生工具结果；立即返回的 Coterm 操作凭据、后续结果查询与 MCP 元数据如何配合，尚待冻结。当前 MVP 将凭据放在 `_meta["coterm/operation"]`，未结束时返回 pending，终态保留插件原生 CallToolResult；正式线缆仍未冻结。
4. **MCP 物理连接**：同进程独立逻辑通道还是与运行时复用 stdio，正式方案尚待选择；MVP 的实例级 Tools 与秘密交互复用后端控制 RPC，对外以固定本机 Streamable HTTP 提供入口；旧 stdio 适配器保留用于回归测试。秘密物理隔离同样待定。
5. **流控制**：本轮提出块数+字节额度、显式补充窗口、暂停/丢弃并报告缺口/关闭三种策略。控制帧必须能在数据拥塞时继续投递，具体调度不在本轮实现。

帧头、错误码清单、安装/市场、签名验证、凭据永久存储、通用组件具体属性和插件 CLI 尚未定稿，本轮不创建虚假的实现或冻结协议。旧 crate 根模块的终端类型属于原型，新契约位于 `coterm_proto::runtime`；评审后再决定迁移。
