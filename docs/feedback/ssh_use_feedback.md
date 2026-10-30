# SSH MCP 未解决反馈

核对日期：2026-10-30。已确认修复与验证记录移入 [SSH 开发进度](../progress/ssh-plugin.md)；本文件只保留尚需现场复测的问题。

## 1. 共享 PTY 复合命令空输出：现场原因待验证

**原始现象**：在部署 open-ai-canvas 时，通过 `ssh.command` 执行 `cd /home/liu/open-ai-canvas && ls -la`，观察到退出码 0，但返回的 `structuredContent.text` 为空或缺少预期列表。

**本次核对**：已修复常见 ANSI 输出转为 Base64 导致 `text` 缺失，以及独立 exec 文本摘要遗漏 stdout/stderr。复合命令、目录切换、提示符变化和逐个分包位置的 PTY 边界回归通过；目前没有证据证明 `MAX_PRE_MARKER` 或回显剥离会在该现场吞掉有效输出。

**待验证**：使用更新后的插件在原远端复测，区分字段缺失、捕获截断和真实无输出。如果仍发生，保留完整 `structuredContent`（含 `encoding`、`data`、字节统计和截断标志）、同游标区间的 `ssh.read` 回放、远端 Shell 类型及提示符设置，确认输出落在哪个边界后再修复。非交互式文件查看优先使用 `ssh.exec` / `ssh.read_file`；不要只检查 `text` 就判定没有输出。

实现入口：[PTY 边界与回显](../../plugins/plugin-ssh/src/command.rs)、[输出编码和限制](../architecture/ssh-plugin.md#命令与取消)。
