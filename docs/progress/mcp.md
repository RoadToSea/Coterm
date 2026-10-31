# MCP Bridge

## 当前能力

- 工具由已启动插件的握手注册，Bridge 保留标准 inputSchema，统一处理实例名称、Agent 来源、审批和操作收据。
- 插件可声明工具 descriptor 的 `_meta["coterm/returnOnApproval"]`，复用运行时状态订阅，在人工审批就绪时返回；未声明的工具和普通运行时提交保持原有等待行为。
- 串口命令可在一次调用中发送文本并收集提示符前的回显，结果同时提供可读文本与结构化字段；近期输出另有无状态快照接口。

## 2026-10-31：串口 MCP 使用反馈

已实现：

- 实例工具描述明确 `name` 来自 `coterm.list_instances`；Bridge 兼容 `instance`、`instanceName`、`instance_name`，拒绝同时传多个别名。
- `serial.command` 的默认行结束符为 CR，显式 `lineEnding:"none"` 保留原样输入；人工单次输入和 HEX 不受此默认值变化影响。
- 命令等待输出在 `content[].text` 中直接可读，同时保留 `structuredContent.output`；等待期间支持取消、断开。
- 新增 `serial.get_recent(lines)`，默认 50 行，范围 1～200，返回最近缓冲的快照，包含连接、游标、缺口及截断标记。
- 回显和近期快照按 JSON 转义后的 6 KiB 文本预算截断，两个 MCP 副本合计满足宿主 16 KiB 结果上限。
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
- 协议、边界与示例见 [MCP 接入](../architecture/mcp-integration.md) 和 [串口插件](../architecture/serial-plugin.md)。
- 继续验收从 [待确认反馈](../feedback/serial_mcp_use_feedback.md) 及 `plugins/plugin-serial/tests/serial.rs` 的真实进程回归入口开始。
