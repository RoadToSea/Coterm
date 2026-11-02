# MCP Bridge

## 当前能力

- 工具由已启动插件的握手注册，Bridge 保留标准 inputSchema，统一处理实例名称、Agent 来源、审批和操作收据。
- 对外目录使用扁平 `coterm_<plugin>_<tool>`；`coterm.status` 提供有界健康 / 连接快照，实例工具在唯一确认已连接且无未知状态时可省略 `name`。
- 插件可声明工具 descriptor 的 `_meta["coterm/returnOnApproval"]`，复用运行时状态订阅，在人工审批就绪时返回；未声明的工具和普通运行时提交保持原有等待行为。
- 串口命令可在一次调用中发送文本并收集提示符前的回显，结果同时提供可读文本与结构化字段；近期输出另有无状态快照接口。

## 2026-11-02：Agent 健康检查、默认路由与扁平工具名

- 新增 `coterm.status`，一次返回桌面 / Bridge 可达、实例 ready 与真实连接状态；最多采样 64 实例、8 并发，整体收集最多 1 秒，未能确认时返回 null。成功响应才能证明可达，关闭的桌面 / Bridge 仍由 Stdio 明确报连接失败。
- SSH / Serial 的 status descriptor 声明 `coterm/connectionStatus`；基座只接受实例级、read、非敏感、transient 的单一状态工具，无需协议硬编码或认证。第三方未声明工具保持未知。
- 实例 `name` 从公共 schema 必填项移除；仅目标插件唯一确认已连接、无未知且快照完整时默认路由。无连接、多连接、未知或显式无效参数不派发写入；查询 / 取消省略目标时使用全局同样约束，仍验证记录归属。
- 目录发布 `coterm_ssh_exec` / `coterm_serial_command` 等扁平名，旧 `<pluginId>__<native>` 仍可调用但不重复列出；归一化碰撞明确拒绝。插件内部操作和进程协议不变。
- SSH 与 Serial 一样，进入人工审批立即返回 `pending_approval`，批准后查询原操作；原有审批、Bypass、密码 / sudo、取消和断线不重放规则保持。
- 宿主每条输入继续限 16 KiB，输出独立限 192 KiB；适配 SSH 两流各 32 KiB 和可读摘要，超限 / 脱敏保护仍有效。Serial 自身预览不扩容。
- 决策与使用：[MCP Agent 调用体验](../design/mcp-agent-experience.md)；稳定契约：[MCP 接入](../architecture/mcp-integration.md)。已解决反馈从原反馈文件移除。

本轮 Windows 验证：

- `cargo check --workspace` 通过。
- `cargo test -p coterm-mcp -p coterm-plugin-ssh -p coterm-plugin-serial`：101 项通过；2 项原本要求显式设备的串口测试保持忽略。
- Core 完整测试出现 5 项插件安装 fixture 失败：当前工作区 Local manifest 新增 frontend，而既有安装测试包仅写入 backend，触发 `plugin_entry_unavailable`。没有改动该 manifest 或混入 Local 开发；明确排除这 5 项后，其余 37 项 Core 测试通过，合计 138 项相关测试通过。
- `cargo clippy -p coterm-core -p coterm-mcp -p coterm-plugin-ssh -p coterm-plugin-serial --all-targets -- -D warnings` 通过。
- `pnpm plugin:ssh:build` / `pnpm plugin:serial:build` 通过，开发插件包已重建；没有关闭运行中的桌面或插件。启动修复版本需重启桌面，让 MCP 客户端重连并刷新目录。

## 2026-10-31：串口 MCP 使用反馈

已实现：

- 实例工具描述明确 `name` 来自 `coterm.list_instances`；Bridge 兼容 `instance`、`instanceName`、`instance_name`，拒绝同时传多个别名。
- `serial.command` 的默认行结束符为 CR，显式 `lineEnding:"none"` 保留原样输入；人工单次输入和 HEX 不受此默认值变化影响。
- 命令等待输出在 `content[].text` 中直接可读，同时保留 `structuredContent.output`；等待期间支持取消、断开。
- 新增 `serial.get_recent(lines)`，默认 50 行，范围 1～200，返回最近缓冲的快照，包含连接、游标、缺口及截断标记。
- 回显和近期快照按 JSON 转义后的 6 KiB 文本预算截断，两个 MCP 副本合计满足当时宿主的 16 KiB 结果上限；2026-11-02 宿主输出预算扩大，串口预览保持原限额。
- 严格 schema 校验仍启用；未知字段和缺失必填字段的错误提示包含字段名，不包含参数值。
- 串口提交工具进入审批时返回 `pending_approval` 和操作 ID，用户批准后查询原操作，不能重提同一写入。

验证入口：

```bash
cargo test -p coterm-core -p coterm-mcp -p coterm-plugin-serial
cargo check --workspace
cargo test --workspace
```

覆盖命令回显、默认 CR、原样输入、实例别名及冲突、未知字段脱敏、审批及时返回、控制字符的结果限额、等待输出时取消。真实串口和虚拟串口配对用例仍需明确选定设备后执行；普通测试不会打开物理端口。

本次验收结果：

- `cargo check --workspace` 通过。
- `cargo test --workspace --exclude coterm-desktop` 通过，覆盖 Core、MCP、串口、SSH、Local 和协议契约；SSH 原有跨审批等待行为回归通过。
- `cargo test -p coterm-desktop --bin coterm-desktop` 的 9 项桌面单元测试通过。
- `cargo clippy -p coterm-core -p coterm-mcp -p coterm-plugin-serial -- -D warnings` 通过。
- `pnpm plugin:serial:build` 通过，已重建串口后端和前端开发包。
- `cargo fmt -p coterm-plugin-serial -- --check` 和 `git diff --check` 通过。
- 完整 `cargo test --workspace` 因运行中的桌面应用锁定 `target/debug/coterm-desktop.exe`，无法完成桌面集成目标的重建；未关闭用户进程。退出旧桌面后需补跑该命令。

启动修复版本时先退出旧桌面，再运行 `pnpm tauri:dev`；串口插件开发包已生成，MCP 客户端需重新连接以刷新工具目录。

## 限制与后续入口

- 客户端导出 `parameters:null` 尚不能在服务端复现：服务端 `tools/list` 已提供标准 schema；仍需对比使用现场的原始响应与客户端转换结果，不能将客户端导出字段当作服务端缺失的证据。
- `serial.get_recent` 是重复可读的尾部快照，不提供隐式共享游标。需要精确增量时使用 `serial.read(cursor)`。
- 基座协议与边界见 [MCP 接入](../architecture/mcp-integration.md)；串口接口设计和使用说明已整理到插件自己的 [docs 目录](../../plugins/plugin-serial/docs/README.md)，[原架构入口](../architecture/serial-plugin.md) 保留基座集成边界与导航。
- 继续验收从 [待确认反馈](../feedback/serial_mcp_use_feedback.md) 及 `plugins/plugin-serial/tests/serial.rs` 的真实进程回归入口开始。
