# SSH 插件开发进度

## 当前能力

- 多实例 SSH、共享 PTY、人机共用终端、独立 exec 作业、按游标读取输出。
- MCP 以实例 `name` 路由；文件读写作为显式实例工具发布；人工终端不使用 Agent 命令列表界面。
- `ssh.exec` 的 `commands` 接受多行脚本，总量 64 KiB UTF-8 字节，通过 stdin 交付，列表项共享目录/环境，首个非零状态停止。
- `ssh.exec`、`ssh.read_file`、`ssh.write_file` 支持显式 `sudo: true`，审批后由桌面收集一次性秘密；不接受 Agent 传密码。
- ANSI 终端文本保留控制字符并直接返回 UTF-8，二进制、无效 UTF-8 或控制字符过密时无损返回 Base64；摘要同时呈现 stdout/stderr，限制 JSON 扩张。

## 2026-10-30：使用反馈修复

- 核对发现当前文件工具已经注册，MCP exec 已自动使用 Shell；增加真实 `tools/list` 和工具调用测试，避免把旧插件/客户端缓存现象认定为源码缺失。
- 移除 exec 的单行 / 4096 字节约束；保留共享 PTY 的物理单行限制，错误信息指向 exec 或文件工具。新增 64 KiB 且大量引号的脚本边界测试。
- 修复 ANSI 导致普通彩色输出缺少 `text`，以及带退出码的 exec 摘要遗漏 stdout/stderr。PTY 分包回归覆盖复合命令、目录切换和提示符变化，不把未复现的过滤错位当成已确认根因。
- exec / 文件结果标识执行通道、是否继承 PTY 状态及 sudo。显式提权使用现有宿主秘密交互，密码仅在收到远端 sudo 提示后送入 stdin；免密时不发送密码行，避免破坏文件内容。
- 修复基座秘密交互恢复为异步作业时重复迁移 `Running` 的错误。保持原操作 ID，取消/过期/重连后不重放写操作。
- exec 输出每流每页 4 KiB，缓冲块 4 KiB，避免两条流合并后超过宿主 16 KiB 记录预算；原始字节计数和缺口语义不变。
- 桌面和独立 Stdio 适配器支持 `--addr`，优先于可选环境变量；默认端口仍是 `127.0.0.1:51234`，不需要 Token，参数错误只输出 stderr。

## 验证与入口

- 本轮本地结果：相关 82 项测试通过，工作区 check / Clippy 通过，SSH 开发包构建通过；未连接原反馈的真实远端。
- 回归命令：`cargo test -p coterm-plugin-ssh`、`cargo test -p coterm-mcp`、`cargo test -p coterm-core`。
- 工作区检查：`cargo check --workspace`、`cargo clippy --workspace -- -D warnings`。
- 插件构建：`pnpm plugin:ssh:build`；更新后重启插件/桌面并让 MCP 客户端刷新工具列表。
- 关键实现：[Backend](../../plugins/plugin-ssh/src/backend.rs)、[PTY 边界](../../plugins/plugin-ssh/src/command.rs)、[Session](../../plugins/plugin-ssh/src/session.rs)、[MCP 摘要](../../plugins/plugin-ssh/src/result.rs)。
- 关键验收：[真实插件进程与本地 SSH 服务](../../plugins/plugin-ssh/tests/ssh.rs)、[Stdio 进程](../../crates/coterm-mcp/tests/stdio.rs)。sudo 测试使用真实 POSIX Shell 与测试认证策略，不修改本机管理员权限。
- 契约见 [SSH 架构](../architecture/ssh-plugin.md) 和 [显式提权设计](../design/ssh-explicit-sudo.md)。

## 限制与下次入口

- 尚未连接反馈中的真实远端复测；共享 PTY 的现场空输出问题保留在[反馈](../feedback/ssh_use_feedback.md)，需要原始回放与实际返回值才能进一步归因。
- 独立通道不复制人工 PTY 的 `sudo`/`su`、目录和环境；需要显式 sudo 或在脚本内设置环境。
- 目前仅支持 sudo 单次密码认证和 NOPASSWD；TTY/MFA/自定义交互、Windows 远端 Shell 不在当前提权实现范围。失败或结果未知均不自动重试。
- 保留既有 8 KiB 文件读取、64 KiB 文件写入、120 秒 PTY、30 分钟 exec 和有界输出限制；后续大文件能力应单独设计分页/传输接口。
