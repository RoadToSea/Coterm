# 项目进度与缺口评估

评估日期：2026-10-26。依据：`docs/`、实际源码、各功能分支的提交差异及本次验证。本文区分「已有代码」「自动化验证」和「实机验收」，不以历史交付报告代替当前测试。

## 总体判断

Coterm 已超过项目骨架阶段：桌面基座、插件实例、SSH、人工审批、秘密输入及 MCP Tools 已形成基础闭环；串口、宿主事件订阅、坏包隔离和响应式界面也已有实现。当前处于**第一阶段收尾、第二阶段运行时完善的早期**，还不能视为稳定的跨平台插件平台或可发布的插件生态。

不提供整体完成百分比：阶段验收清单此前只有目录骨架，长期目标还包含市场、签名、多协议和工业场景，缺少可比较的工作量基线。

## 能力与实现证据

| 模块 | 当前状态 | 证据与边界 |
|---|---|---|
| 桌面容器与插件发现 | 已实现 MVP | `apps/desktop/src-tauri/src/main.rs`、`crates/coterm-core/src/plugin_manager.rs`：逐包诊断、坏包隔离、已安装包优先、生命周期、多实例和前端挂载 |
| 操作与审批 | 已实现 MVP | `plugin_manager.rs`、`operation_history.rs`：可信来源、scope/effect、schema 校验、提案快照、显式 Bypass、查询和取消；历史有界但不持久化 |
| 宿主交互与界面 | 已实现主要路径 | `HostApprovals.tsx`、`HostInteractions.tsx`、`host-api.ts`：共享 attention 订阅、多提案、密码交互队列及迟到快照保护；视觉 token、窄窗布局已有浏览器用例 |
| SSH | 已有协议闭环 | `plugins/plugin-ssh`：密码认证、指纹确认、共享 PTY、命令输出/退出码、回放和事件；取消只能尽力发送 Ctrl+C，不能证明远端停止 |
| 串口 | 已有初版，存在可靠性及跨平台缺口 | `plugins/plugin-serial`：枚举、连接、字节读写、HEX/行结束符、有界回放、独立前端；MCP 可读及连接/断开，不开放字节写入 |
| 本地终端 | 只有运行时骨架 | `plugins/plugin-local/src/backend.rs` 已迁移当前 manifest 和 `PluginBackend`，仅提供 `local.plugin_info`、实例及配置校验；没有实际 PTY 或命令执行 |
| MCP Bridge | Tools 路径可用 | `crates/coterm-mcp`：固定本机 HTTP、实例发现、插件命名空间、调用等待/查询/取消、目录变化通知；旧 stdio 适配器保留作开发回归 |
| SDK 与通用组件 | 已有内部实现，契约未冻结 | `packages/plugin-sdk` 有 TypeScript 契约和可选块列表；Rust 共用 `runtime/transport.rs` 行协议；尚非稳定发布 SDK |
| 流与公共交互 | 部分仅为接口声明 | `createPluginAPI()` 的 `subscribeStream`、`requestText`、`requestSelection` 明确抛出未接通错误；确认仍用 `window.confirm`，秘密输入已接通 |
| 命令块与上下文 | 组件可用，产品闭环未完成 | SDK 块列表已测试，但 SSH 前端尚未接入命令块选择与 MCP 上下文；旧 OSC 133/终端组件不能当成当前插件链路已完成的证据 |
| 其他协议与生态 | 未实现 | 未见 MQTT、CAN、Modbus 实现，也未见插件市场、签名验证、更新回滚和正式安装分发流程 |

## 优先补齐的部分

### P0：发布前的可靠性与边界

1. **串口关闭与线程回收**：`Connection::drop` 只 abort Tokio 写入任务，独立读取线程没有显式停止信号或 join；设备空闲时读超时会持续循环。打开端口也在没有 await 的 async 块内直接执行同步 `.open()`，外层 timeout 不能使阻塞调用变为可取消。需补断开、销毁实例、停止插件、重复连接的端口释放与任务退出测试，确认端口和通知发送端均能释放，并将阻塞打开操作隔离。
2. **串口原始输入脱敏**：`serial.write` 当前 `sensitive_input=false`，但人工串口终端可能输入登录密码；应明确与 SSH 原始输入一致的账本脱敏边界，并验证秘密不会出现在可查询操作参数中。远端回显属于另一条边界，不能靠输入脱敏承诺消除。
3. **跨平台串口接入**：`Config::validate` 拒绝所有 `/`，包括 Linux/macOS 的 `/dev/ttyUSB0`、`/dev/cu.*`，但 manifest 声明这些平台。需按平台和设备路径建立校验规则；同时验证 Unix 端口大小写/别名、系统依赖和设备权限。
4. **建立可执行验收基线**：补齐阶段验收矩阵，明确自动测试、真实 Tauri、远程 SSH 和硬件串口的证明范围。浏览器 IPC 替身不代表真实桌面端到端测试；本次将串口硬件用例改为显式 ignored，并移除普通基座回归中默认连接/写入 COM6 的路径，避免假通过和触碰真实设备。

### P1：稳定运行时与可交付工程

- 冻结 manifest、错误码、消息编码、版本兼容和能力协商，统一开发指南中的草案与当前实现；发布最小插件模板与契约测试工具。
- 接通通用二进制流/StreamRouter、窗口信用、背压、流取消和缺口恢复；用持续订阅型插件验证，而不只测试终端事件。
- 完成文本/选择交互、多轮交互与 MFA，以及插件崩溃、重启后的恢复或明确失效；保持写操作不自动重放。
- 明确实例配置、布局、操作历史和插件业务数据各自的持久化责任，补应用重启后的行为。当前历史仅在内存。
- 建立 Windows/macOS/Linux CI、可重现依赖与发布打包：当前无 `.github` 工作流，`Cargo.lock`、`pnpm-lock.yaml` 被忽略；构建脚本只产出当前平台插件。macOS 图标文件也需核验，当前 `icon.icns` 只有 8 字节。
- 补真实桌面启动、安装包内插件发现、打包后的资源路径、升级和退出回收验收；不能把 `pnpm build` 等同于 `tauri:build`。

### P2：产品与生态扩展

- SSH 私钥、ssh-agent、keyboard-interactive/MFA、known_hosts、SFTP、端口转发与重连。
- 本地 PTY 完整插件；串口流控、DTR/RTS、热插拔与自动重连；后续 MQTT/CAN/Modbus 插件。
- 插件自有命令块到可选择上下文的完整路径，以及 MCP Resources、Prompts、订阅、进度和协议级取消。
- 插件来源信任、签名、权限、前端隔离、秘密物理通道、更新回滚和市场。同 WebView 和子进程目前均不等于安全沙箱。

## 文档对齐

- `SPEC.md` 的插件所有权与最小基座方向仍适用。
- 根 README 将所有协议描述为现成特性，并声称 Token 降低 90% 以上，缺少当前验证；本次调整为实际能力和限制。
- 第一阶段此前仅有待编写占位；本次补充实际交付与待验收矩阵，避免继续把已有实现描述为尚未选定协议。
- `interface-review.md` 中「本地插件尚未迁移」与串口分支代码不一致；本次更正为已迁移骨架、未实现 PTY。
- 运行时设计文档中的「主动事件尚未实现」已过时；已存在有界 JSON 事件，但正式二进制流仍未接通。
- WS-13 文档保留为历史验收记录，其中测试数和提交分支属于当时记录，不代表本次合并后的结果。

## 本次分支归并

清理前，远端默认 `main` 只有独立根提交及 `LICENSE`；`develop`/本地 `master` 停留在脚手架；实际功能分散在串联 feature 分支及 agent 分支。

- 以 `agent/deepseek/ws-13` 的本地末端 `08e4645` 为集成基线，保留 WS-10、WS-11、WS-13 及先前全部功能历史。
- 合并 `origin/main` 的独立根历史，保留原始 LICENSE；不重写或强推历史。
- 合并 `agent/planned-task-executor/ws-8`（`018d19b`）的串口、本地插件骨架和共享传输层。唯一文本冲突是 `list_plugins`：保留 WS-13 的 `PluginCatalog` 返回值与逐包诊断，它已涵盖串口分支的坏包跳过意图。
- `agent/mika/f23f01335413`、`agent/mika/ws-8`、`agent/ui/ws-8` 分别是任务描述、审查副本/复现脚本、图片附件，不是独立产品实现。使用本地 `archive/20261026/*` 标签保留原提交后清理分支，避免把嵌套仓库副本导入产品目录。审查副本中的旧快照复现已有 WS-13 正式回归用例覆盖。
- 最终保留 `main` 和 `develop`，两者指向同一已验证集成结果；已包含的功能/agent 分支和重复 `master` 清理。另一个干净 worktree 保留文件，解除其对已合并分支的占用。

## 本次验证

环境：Windows x86_64，Cargo 1.97.0、Node.js 22.22.2、pnpm 11.0.8。结果均来自本次集成工作区，而非历史分支报告。

| 命令/检查 | 结果 |
|---|---|
| `cargo check --workspace` | 通过 |
| `cargo test --workspace` | 64 passed、0 failed、1 ignored（显式硬件测试） |
| `cargo clippy --workspace -- -D warnings` | 通过，无告警 |
| `pnpm install --no-frozen-lockfile` | 非交互环境设置 `CI=true` 后通过；锁文件仍遵循现有忽略规则 |
| `pnpm build` | 桌面 TypeScript 与 Vite 构建通过 |
| `pnpm --filter coterm-plugin-ssh-ui build` | 通过 |
| `pnpm --filter coterm-plugin-serial-ui build` | 通过 |
| `pnpm test` | 26 passed、0 failed、0 skipped，包含真实浏览器测试 |
| 修改文档的本地链接、`git diff --check` | 通过 |

集成时修复了此前未通过的测试设施：本地插件用例漏传 `args`、插件级目标错误地传空字符串、运行时平台写成 `test`、并行测试共用临时包目录，以及把宿主受理前的结构化错误误断言为操作终态。定向复现先得到 1 passed/3 failed，修复后 4/4 通过，随后全工作区通过。生产接口未为迎合测试而修改。

串口普通回归不再默认连接/写入 COM6，MCP 实例绑定断言也不再依赖测试机恰好存在串口。硬件测试需显式 `--ignored` 及设备配置，默认不会触碰设备。

本次未运行真实 Tauri 窗口、远程 SSH 或物理串口验收，也未运行全平台安装包构建。自动测试通过不消除本文列出的产品缺口。详细执行日志与清理前 Git bundle 保存在本机 `.git/coterm-validation/`，不纳入产品源码；归档标签在本地保留，产品提交同步远端。
