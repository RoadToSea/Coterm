# 实现与验收状态

本文件记录已实现能力及最近有效验收；未解决的使用反馈、性能失败证据和后续调查见[feedback](../feedback/known-issues.md)。行为要求见[需求](../requirements/)，复测命令见[开发与验收](development.md)。

## 当前能力

| 模块 | 已有能力 | 主要边界 |
|---|---|---|
| 工作区与配置 | 保存项、分组、布局、恢复、规则组引用与实例 Bypass | Windows 加密保存；其他平台工作区仅内存；恢复不连接 |
| 实例运行 | 五种进程内驱动、多实例、统一唯一名称、类型化调度、异步状态与故障收敛 | 每驱动任务隔离；离线保存项也占用名称 |
| 审批与交互 | 独立规则组、实例单组绑定、正反向规则、独立 Bypass 和秘密交互 | 已待审批提案不随规则更新自动执行；通用多轮认证未完成 |
| 通信驱动 | SSH／Local 前台与显式后台，Serial Shell／stream，Telnet 与 Raw TCP | SSH 认证和 Shell 范围有限；Telnet／TCP 明文，代理无认证 |
| 数据记录与读取 | 后端有界正文、操作摘要、三种无游标读取、前端回放 | 数据不跨启动；字节流无对话；长期慢消费可产生不可恢复缺口 |
| 桌面交互 | 宿主 React 配置与两类工作台、统一主题控件、分组、标签、终端、字节调试、横向命令块、审批设置 | 原生跨平台、完整后代进程回收和现场协议兼容仍需验收 |
| Agent 接入 | 进程内 Streamable HTTP、8 个统一语义工具、名称路由、多客户端并发与操作查询 | 仅本机、无 Agent 身份隔离；无 Resources／Prompts／事件订阅／MCP cancellation notification |

## 最近有效的软件验收

Raw TCP 服务端：`cargo check --workspace` 和 `cargo test -p coterm-driver network:: -- --test-threads=1` 通过，14 项网络驱动测试覆盖监听启动、双向收发、客户端断开后继续监听、连接身份更新以及显式停止后端口可重新绑定。服务端会话文件通过 rustfmt 检查。

软件检查：`cargo check --workspace --all-targets --locked`、`cargo test --workspace --locked -- --test-threads=1` 和 `cargo clippy --workspace --all-targets --locked -- -D warnings` 通过。完整 Rust 回归 287 项通过，3 项设备／虚拟串口用例按默认设置 ignored；包含 36 项桌面后端测试、7 项统一 MCP 集成测试、27 项 SSH 集成测试和 7 项 HTTP 协议测试。`pnpm build`、`pnpm test:build` 和完整 `pnpm test` 通过，105 项前端测试通过，无失败、无跳过。

HTTP 协议与隔离：初始化、固定八项工具、通知 202、无会话调用、Host／Origin 和媒体类型、非法 JSON／请求／协议版本均有回归。恰好 1 MiB 的有效 JSON 可接收，多 1 字节返回 413；128 个请求占满后返回 503，释放请求容量后继续服务；重复 Origin 被拒绝。关闭服务会结束空闲连接并释放端口。两个独立客户端可使用相同 JSON-RPC id 对不同实例产生独立审批；跨实例查询被拒绝，取消其中一个不改变另一个提案。

Windows 原生与 SDK 互通：隔离 smoke 构建使用真实 WebView2、Local PowerShell、回环 TCP／Telnet 和 MCP HTTP，已验证审批 UI、命令与后端结果、前台状态延续、保存恢复不重连、不同实例并发查询、配置 URL、换端口与服务启停。保持 HTTP 空闲连接后发送真实关窗请求，桌面自行退出，连接关闭且端口可重新绑定；强杀不作为通过依据。官方 TypeScript MCP SDK 1.32.2 独立验证版本协商、工具目录、双客户端审批、实例归属与取消互不影响；SDK 只临时安装于调试目录，未加入项目依赖。旧 `mcp stdio` 参数的集成测试确认立即以 2 退出、stdout 为空且 stderr 提示 HTTP 迁移。MCP 设置保持 version 1，工作区保存格式未修改。验收后已恢复普通 debug 构建，确认桌面 EXE 无进程占用、隔离 profile 已删除。

文档与界面检查：21 份 Markdown 的 175 个本地链接及锚点、`git diff --check` 通过；已检查窄窗口设置页截图。HTTP 配置弹窗由原生验收核对 URL 和生成的客户端配置，未扩大为完整原生外观验收。

现有业务回归覆盖：

- 工作区隔离夹具验证多次重启、仅恢复 A 时离线 B 仍保留、新建不清除旧项、活动保存身份、显式销毁／忘记、容量拒绝、写失败回滚、审批组及 Bypass；v5 加密编码保留原 label 键。旧文件已有重名时保留配置并拒绝冲突恢复。
- 运行时和统一工具验证名称去空白、跨驱动重名拒绝、UTF-8 边界、销毁后复用、当前能力、无游标读取、未知参数及内部身份拒绝、Serial／TCP 参数边界和 Telnet HEX 拒绝。多个驱动连接时，省略名称不能按驱动偷偷选择目标。
- 本机 SSH 服务验证认证秘密、统一文件写入审批、独立读取通道、多行 exec 首错停止、64 KiB UTF-8 正文往返、JSON 编码预算、30 ms 超时、16 秒慢操作及并行查询、运行中取消和断连后作业收敛。Local 覆盖 Windows PowerShell 与 cmd；没有退出码的关闭保持 unknown。
- 浏览器覆盖五种驱动配置、TCP client/server 配置及监听状态、串口端口刷新与保存身份、终端／字节视图、UART 暂停期间继续读取和计数但丢弃显示数据、恢复后不补显、分片行时间戳、接收文本原生选区复制与输入框快捷键、命令块选择与复制、搜索与快捷键、SSH 任务和认证、主题／语言／字号、动态圆角、宽窄窗口及卸载重挂载。Local 生命周期包含首次启动、真实退出码、重新挂载、丢弃旧 shellId 事件及保留当前输入错误。HTTP 设置支持复制 URL 配置、启动策略、端口校验与失败状态。

## 验收边界

- 未重新进行真实开发板收发或吞吐测速；高速串口和慢消费者的失败证据仍见 [feedback](../feedback/known-issues.md)，不能据软件回归宣称无损速率提高。
- 原生验收使用隔离配置，不读取现场工作区，未覆盖原远端 SSH、实际代理、真实 Telnet 设备、干净 NSIS 安装或 macOS／Linux。完整外观矩阵见[前端待验收项](../feedback/ui-theme-audit.md)。
- 工作区容量回归曾有一次未定位的写入失败，当前完整回归通过不等于已证明根因消失；已被覆盖的现场配置恢复同样仍见 [feedback](../feedback/known-issues.md)。
