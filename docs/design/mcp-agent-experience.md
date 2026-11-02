# MCP Agent 调用体验

日期：2026-11-02。状态：已实现；接口契约见 [MCP 接入](../architecture/mcp-integration.md)，验证记录见 [MCP 进度](../progress/mcp.md)。

## 决策与边界

| 反馈 | 决策 | 保留的边界 |
|---|---|---|
| 缺少探活 | 新增无参数 `coterm.status`，返回桌面 / Bridge 可达、实例 ready 与协议连接状态 | 服务离线时依然由 Stdio 报连接失败；最多 64 实例、8 并发、1 秒收集，未知状态明确为 null |
| 每次必须查询实例名 | 实例工具 `name` 可选，目标插件唯一确认已连接实例可默认路由 | 零个 / 多个连接、未知、超限或显式无效参数时拒绝猜测；连接操作仍显式指定未连接实例 |
| 人工审批等待 60 秒 | SSH 和 Serial descriptor 声明 `returnOnApproval`，首次审批收据立即返回 `pending_approval` | 不跳过审批、认证、sudo 秘密输入或来源校验；查询原操作，禁止自动重放 |
| exec 4 KiB 频繁分页 | 预览和输出页增至每流 32 KiB；普通文本可读摘要跟随扩容；宿主输出记录独立增至 192 KiB | 每流仅保留 64 KiB、最多 8 作业；输入仍为 16 KiB、MCP 帧 1 MiB；大输出仍有分页和缺口 |
| 双重嵌套工具名称 | 目录发布 `coterm_ssh_exec` / `coterm_serial_command` 等扁平名称 | 旧名称作为未列出的兼容别名；碰撞拒绝；内部操作和进程协议不变 |

连接知识通过插件工具 descriptor 的 `coterm/connectionStatus` 显式声明。Bridge 只调用实例级、read、非敏感、transient 的单一声明工具并读取 `structuredContent.connected`；不在基座硬编码 SSH / Serial 协议，也不把 ready 等同于已连接。未声明的第三方插件仍使用显式 `name`。

## Agent 使用顺序

1. 可选调用 `coterm.status({})`，查看实例和连接状态。
2. 已有唯一已连接 SSH 时，直接 `coterm_ssh_exec({"commands":["docker ps"]})`；多目标时添加 `name`。首次连接使用显式 `name` 的 probe / connect。
3. 返回 `pending_approval` 时提示用户到桌面审批，保存 `operationId`，使用 `coterm.get_operation({"name":"原实例名称","operationId":"原 ID"})` 查询；不重复 exec。
4. 完成结果优先读 `structuredContent.stdout/stderr`，文本客户端可直接读 `content[].text`；只有预览截断或任务尚未完成才按需调用 `coterm_ssh_output`，检查 `gap/more` 并使用服务端返回的游标。

查询 / 取消可省略目标，但只在全局唯一确认已连接且没有未知状态时成立。长任务期间目标可能断开或新增其他实例，因此多实例和跨连接状态查询建议始终保留原 `name`。

## 验收

- 真实插件进程 / MCP 连接覆盖健康快照、未连接不默认、单连接默认、第二个未连接不干扰、多连接拒绝、错误别名 / UUID 拒绝和旧名称兼容。
- 审批收据在 500ms 测试界限内返回；批准后查询原操作得到最终结果，同一 MCP 连接可继续并发读状态。
- 本地 SSH 测试服务执行真实 POSIX Shell，验证 32 KiB 输出在首次 MCP 返回、可读文本和操作查询中完整保留；ANSI / UTF-8 / 二进制大小计费、两流预算与宿主超限保护另有单元测试。
- 无新增依赖、Tauri 权限、进程启动方式或跨进程消息；Windows 执行本轮测试，macOS / Linux 的逻辑和字节限额共用，但仍需平台实机验收。
