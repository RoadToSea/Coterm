# 5. 代码架构

阅读前提：[模块架构](01-modules.md) → [模块需求](02-requirements/) → [公共需求](03-common-requirements.md) → [模块交互](04-module-interactions.md)。

本文件先确定整体依赖，再安排公共能力，最后描述模块内部实现。标为“当前”的内容来自现有代码；“后续建议”是尚未实施的整理方向，不代表接口或持久化结构已变更。

## 一、整体组织与调用链

### 当前布局

```text
apps/
  desktop/src/                  React 桌面、宿主接口、实例视图
  desktop/src-tauri/src/        应用组合、Tauri 命令、工作区与 MCP 设置
  shared/                      应用内部前端契约、终端与字节工作台
crates/
  coterm-core/src/
    model/                     操作、实例、错误与交互数据
    runtime/                   实例调度、驱动任务与状态收敛
    approval_policy.rs         实例审批判断与规则编译
    operation_history.rs       脱敏、有界宿主账本
    terminal/                  有界终端缓冲与编解码
    diagnostic/                显式诊断适配，不在产品调用链
  coterm-driver/src/
    ssh/ serial/ local/        三种驱动
    tcp/ telnet/               两种网络驱动入口
    network/                   TCP 生命周期、代理与网络命令共用实现
    stream/                    串口与网络共用缓冲及提示符观察
  coterm-driver/frontend/      五个驱动的内置前端
  coterm-driver/manifests/     编译期固定声明
  coterm-mcp/src/              工具映射、Bridge 与 Stdio 适配
scripts/                       构建、验收和调试清理
tests/                         跨模块与浏览器验收
docs/                          唯一长文档目录
```

生产依赖方向为桌面组合根依赖 core、driver、MCP；driver 和 MCP 各自依赖 core；core 不反向依赖具体驱动或桌面。驱动测试对 MCP 的开发依赖不属于生产调用链。

驱动是链接到桌面主进程的 Rust 库；每种驱动独立异步任务，任务内管理多个实例。不是五个外部 driver.exe，也不是五个进程级沙箱。本地 Shell 与 MCP Stdio 适配器按用途仍可使用子进程。

### 控制与数据路径

```text
桌面驱动前端 → 实例绑定 Host API → Tauri 命令 ─┐
                                             ├→ Runtime → 实例审批 → DriverTask → 驱动
Agent → 桌面 EXE 的 mcp stdio → 本机 Bridge ──┘

驱动 → 后端有界数据／命令记录 → read_* 快照 → Runtime → 调用方
驱动 → 独立通知队列 → Runtime 事件 → 视图显示副本
驱动 → 完成通知 → Runtime 更新操作账本 → 原操作查询
```

桌面前端不经过 MCP；驱动控制不经过内部 JSON-RPC；数据正文由驱动直接保留，不先搬进通用 Repository 再读回。每次控制应答与异步通知是不同路径。

固定工厂与前端资源由 [builtin_drivers.rs](../apps/desktop/src-tauri/src/builtin_drivers.rs) 注册。注册校验声明、能力和参数目录，不扫描外部驱动目录、不运行外部入口、不做产品运行期握手。诊断 JSON-RPC 仅供显式示例和契约测试。

### 后续建议

保留现有三个 crate 和现有五种驱动入口。七个逻辑模块不新增七个 crate，也不新增一套 Controller／Service／Repository 空转层。

优先在现有包内整理拥有真实业务判断的代码。桌面命令只处理可信来源、参数转换和调用；Runtime 保留调度、状态和授权；协议规则继续留在驱动。只有重复逻辑和变更点已确认时才提取，不把只调用一次的薄转发层当作模块化成果。

## 二、公共需求的实现位置

| 公共能力 | 当前实现与复用点 | 实现边界 |
|---|---|---|
| 身份、操作效果、错误、收据 | [core/model](../crates/coterm-core/src/model)、[driver.rs](../crates/coterm-core/src/driver.rs)、[前端契约](../apps/shared/driver-api.ts) | Rust 类型用于宿主控制，业务参数仍按各驱动 schema 校验 |
| 来源识别、目标验证与异步操作 | [runtime/mod.rs](../crates/coterm-core/src/runtime/mod.rs) | 同一入口执行审批和状态管理，不让驱动自行授予人工权限 |
| 实例审批 | [approval_policy.rs](../crates/coterm-core/src/approval_policy.rs) | 接收已验证上下文，只判断是否需要审批，不连接或执行 |
| 配置可靠保存 | [workspace.rs](../apps/desktop/src-tauri/src/workspace.rs)、[workspace_approval.rs](../apps/desktop/src-tauri/src/workspace_approval.rs) | 由桌面协调保存与运行态发布，不由各驱动重复落盘 |
| 有界任务与通知 | [runtime/task.rs](../crates/coterm-core/src/runtime/task.rs) | 控制应答、完成通知和可丢失事件分别处理 |
| 宿主操作账本 | [operation_history.rs](../crates/coterm-core/src/operation_history.rs) | 脱敏摘要与状态，不代替驱动原始正文 |
| 终端基础工具 | [core/terminal](../crates/coterm-core/src/terminal) | Local 使用；不要求强制替换 SSH 已有领域缓冲 |
| 字节流缓冲与提示符 | [driver/stream](../crates/coterm-driver/src/stream) | Serial 与网络复用；不包含串口控制线或 SSH 命令包装 |
| TCP 连接与代理 | [driver/network](../crates/coterm-driver/src/network) | Raw TCP 与 Telnet 共用，Telnet 编解码仍独立 |
| 前端终端与字节交互 | [apps/shared/components](../apps/shared/components) | 共用显示和生命周期，不接管协议或建立第二套保存结构 |

公共复用以消除重复知识为准，不要求所有缓冲、命令记录和 Shell 都统一成同一种实现。输入输出字段相似不能证明协议边界、取消或执行语义相同。

### 后续建议

- Runtime 的实例管理、操作状态、交互和审批编排集中在同一大模块，可按现有职责在包内拆分；保留一个外部 Runtime 入口，不让调用方组装内部步骤。
- 桌面的工作区与审批落盘已经有稳定位置，继续由该处负责可靠保存，避免把 IO 搬入纯规则判断。
- 若复用 SSH／Local 记录逻辑，先验证实际相同的状态、预算和取消行为；只抽相同逻辑，不要求迁移为统一持久模型。
- 通用流信用窗口、独立存储服务、插件安装系统没有当前使用需求，不因预留类型存在就建设新基础设施。

## 三、模块内部实现

### 工作区与配置

[workspace_commands.rs](../apps/desktop/src-tauri/src/workspace_commands.rs) 协调创建、恢复、分组和布局；WorkspaceStore 保存版本化配置。Windows 使用当前用户 DPAPI 加密和原子替换，非 Windows 当前只提供内存工作区。

持久文件保持现有格式与兼容读取：工作区写 v5、兼容 v1–v4，文件名仍为 workspace.v1.dpapi；旧 plugin 命名和旧逐实例策略只在兼容读取时迁移。实例恢复保留 configRef，生成新 driverInstanceId 和 sessionId。旧规则按实例独立迁移，不因正文相同自动合并。

配置变更和生命周期使用同一协调锁。组变更先校验、编译、原子保存，再发布新策略；绑定和已保存实例 Bypass 同样先保存后更新。在线模式切换是先变更运行态再尝试保存，失败明确提示，不能假定所有配置变更都有相同事务语义。

### 实例运行

Runtime 拥有实例路由、目录、审批对象、操作状态与交互关联。DriverTask 使用有界请求队列和一次应答通道；长操作返回延迟收据，再以独立完成通知收敛。具体容量和错误见[接口参考](reference/interfaces.md)。

驱动任务退出时，未派发操作取消，已派发而未确认的操作变为 unknown；清理待审批和交互，实例变为 failed。控制超时会关闭该驱动任务，不能与调用方普通等待超时混淆。

### 审批与交互

InstanceApproval 持有编译策略和实例 Bypass，接收 ApprovalContext，判断 Agent commit 是否需要审批。命令文本来自注册 schema 的 x-coterm-approval-text 指针；不是扫描任意文本参数。

审批组增删和分配由 [approval_commands.rs](../apps/desktop/src-tauri/src/approval_commands.rs) 协调；组正文独立持久化，实例仅保存引用。敏感草稿在专用待审批内存保留，普通账本和 MCP 只能拿脱敏内容。

秘密经 ProvideSecret 专用控制路径投递，普通交互结果只带目标、用途、有效期受限的句柄。host_attention 快照以版本通知失效，前端先监听再读；条目消失、过期或实例销毁后关闭对应交互。

### 通信驱动

| 驱动 | 主要内部职责 | 现有入口 |
|---|---|---|
| SSH | 目录与提案、连接认证、共享 PTY／exec、边界、输出和命令记录 | [ssh](../crates/coterm-driver/src/ssh) |
| Serial | 目录与配置、顺序发送、端口句柄、控制线、线程读写与关闭 | [serial](../crates/coterm-driver/src/serial) |
| Local | Shell 解析、PTY、前台命令、后台子进程和记录 | [local](../crates/coterm-driver/src/local) |
| Raw TCP | 固定入口加网络共用实现，透传原字节 | [tcp](../crates/coterm-driver/src/tcp) |
| Telnet | 固定入口加网络共用实现、增量协议协商 | [telnet](../crates/coterm-driver/src/telnet) |

SSH 前台使用同一 POSIX Shell 的语法检查和随机边界包装；后台独立通道。Local 按 POSIX、PowerShell、cmd 分别处理命令，不能直接复制 SSH 包装方式。

Serial 使用阻塞工作线程；Windows 的 [windows_io.rs](../crates/coterm-driver/src/serial/windows_io.rs) 使用 overlapped IO，每方向独立完成事件和期限。读取以有界批次通知，满队列保留最新重试线索，关闭提交尾部。不是暂停接收后写入。

网络按每实例任务维护 socket、连接身份、重连与有界输入。Telnet codec 在正文进入 stream 之前处理 IAC／NVT；Raw TCP 不经过这层协议解释。

### 数据记录与读取

宿主账本与驱动数据各保存一份自己负责的信息。SSH 的 BlockLog、Local 的 History、stream 的 OutputBuffer／ShellHistory 分别维护领域记录。后端数据目前只驻内存，JSON 是字段表示，不表示写入 JSON 日志文件。

前端事件携带连接身份和内部位置，缺口时按协议回放；MCP 读取提供不带消费游标的快照。记录和结果均按实际序列化大小限制，不能只按未转义文本长度判断预算。

### 桌面交互

React 桌面使用 Tailwind 与现有 shadcn 源码组件；驱动前端仍是当前已有的内置模块，不在整理文档时改写框架。driver-loader 按实例挂载可信前端，Host API 固定目标和来源。

| 共享入口 | 当前职责 |
|---|---|
| [terminal-surface.ts](../apps/shared/components/terminal-surface.ts) | xterm、主题、字号、搜索、快捷键、尺寸监听和释放 |
| [stream-workbench](../apps/shared/components/stream-workbench) | 终端／字节视图、回放、发送队列、HEX、历史、定时发送和偏好 |
| [network-connection.ts](../apps/shared/components/network-connection.ts) | 网络配置、回显、状态补取，不含 SSH 文件与 exec 面板 |
| [terminal-blocks.ts](../apps/shared/components/terminal-blocks.ts) | xterm marker、数字块号、选择和上下横线，不识别执行边界 |
| [block-list.ts](../apps/shared/components/block-list.ts) | 有界纯文本块列表和选择；与终端叠层是两个展示入口 |
| [toasts.ts](../apps/shared/components/toasts.ts) | 不抢焦点的有界提示，不替代审批或秘密交互 |

颜色来自桌面语义变量，由 Shadow DOM 继承；终端读取解析后的调色板。切换主题原地更新，不重建连接。尺寸回调只对可见容器执行，卸载清理监听、队列后续动作和计时器。

apps/shared 是内部源码，不是外部 SDK。当前前端契约 2.0 绑定实例；saveConfiguration 可选，subscribeEvents 安装后返回注销函数，处理完成确认消费；预留 subscribeStream 当前返回不支持。

### Agent 接入

[coterm-mcp](../crates/coterm-mcp/src) 只承担 MCP 会话、工具映射、目标名称解析和操作结果包装，直接复用 Runtime，不再增加独立操作服务。

桌面 EXE 的 mcp stdio 模式不创建窗口或驱动运行时，只转发本机私有 Bridge。工具由驱动明确注册，公开 schema 将内部 driverInstanceId 替换为 name；Agent 始终以 Agent 来源提交。

## 四、实现变更的约束

本次文档整理不移动源码、不改变协议、权限、持久化结构或依赖。后续代码调整以[模块交互验收](04-module-interactions.md)和[开发验收](development.md)为依据，保持原调用入口与失败语义；需要改变共享接口或保存格式时另行确认。
