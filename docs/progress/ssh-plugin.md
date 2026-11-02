# SSH 插件开发进度

## 当前能力

- 多实例 SSH、共享 PTY、人机共用终端、独立 exec 作业、按游标读取输出。
- MCP 以实例 `name` 路由；文件读写作为显式实例工具发布；人工终端不使用 Agent 命令列表界面。
- `ssh.exec` 的 `commands` 接受多行脚本，总量 64 KiB UTF-8 字节，通过 stdin 交付，列表项共享目录/环境，首个非零状态停止。
- `ssh.exec`、`ssh.read_file`、`ssh.write_file` 支持显式 `sudo: true`，审批后由桌面收集一次性秘密；不接受 Agent 传密码。
- ANSI 终端文本保留控制字符并直接返回 UTF-8，二进制、无效 UTF-8 或控制字符过密时无损返回 Base64；摘要同时呈现 stdout/stderr，限制 JSON 扩张。
- 后台任务默认单行，从新到旧排列；可展开限高列表，选中后自动收起并将选中项定位到行末。
- 主终端使用 SDK xterm Gutter 显示 PTY 命令块号和选中高亮；Human / Agent 标签在命令前，按钮仅有取消全选。单行 PTY 命令仍可在终端底部输入。
- Exec 胶囊有 LED 状态、右侧 / 底部停靠、可拖动尺寸；历史选择暂停跟随，关闭面板时新任务不自动展开。
- 块选择通过 `ssh.select_blocks` 按当前 epoch 传入插件内存；MCP `mcp.ssh.selected_blocks` 只读取人工选中的有界上下文，Agent 来源不能更改选择。

## 2026-11-02：主终端块选择与 Exec 面板

- 命令起止通过后端可信边界转换为有界 PTY 帧元数据（仅 ID、编号、来源，不带命令或输出），SDK 使用 xterm marker 和 decoration 定位块号；命令显示文字按需从人工块快照读取。UI 标签只写入本地终端显示，不改变 SSH 输入字节。备用屏幕隐藏 PTY 标记，scrollback 淘汰时清理 ID 与选择。
- 块选择只通过实例 / epoch 绑定的 `ssh.select_blocks` 更新；Human MCP Tool `mcp.ssh.selected_blocks` 只读所选不可信数据、明确截断状态和分页信息。重连及卸载清空选择；历史淘汰自动修剪选择。
- Exec 新任务只在详情面板打开且跟随最新时切换；已关闭面板不被自动展开。历史提示、局部 Esc、方向停靠、Splitter 拖动和按实例尺寸保存已接通。
- 文档：[产品约束与验收](../design/ssh-terminal-interaction.md)、[SSH 架构契约](../architecture/ssh-plugin.md)、[SDK 终端行号栏](../../packages/plugin-sdk/docs/command-blocks.md#终端行号栏)。
- 验收：SSH 前端真实浏览器覆盖边界、Human/Agent 标签、实例选择上下文、按键焦点、备用屏幕、scrollback 淘汰、主题、320px 布局；Exec 浏览器覆盖关闭面板、跟随/锁定/恢复、方向停靠、拖动与卸载。真实 SSH 测试覆盖 Human 写选择、Agent 只读、stale epoch 拒绝和原始按键不入上下文。

## 2026-11-01：消息块首次接入

- 后端新增内存块日志和仅人工前端使用的 `ssh.blocks` 分页操作；沿用现有 PTY 随机标记与 exec 通道归属回复，不记录原始按键或秘密交付。
- 块保留真实操作 ID、获批后的命令、可信来源、通道、状态和退出码。未确认结束 / 重置 / 断线标为 unknown，取消和超时单独显示；MCP 列表按一次操作形成一个块。
- 每连接保留最多 32 块，命令预览 1 KiB、stdout / stderr 合计预览 2 KiB，单页 JSON 小于 12 KiB；淘汰与截断可见。事件只携带 epoch / revision，在 tick 合并并处理慢消费者。
- SSH 前端直接导入 SDK `mountBlockList`，保留完整终端、原任务行及输出视图；消息块新增共享 PTY 单行输入，焦点、文本选择、块选择在回复更新时保持。只在块视图可见时读取，错误可重试，缺口重读，成功重连清理旧 epoch。
- 无新增依赖、宿主接口、跨平台启动方式或权限；所选上下文提交给 Agent 仍待后续接入。

实现入口：[后端日志](../../plugins/plugin-ssh/src/blocks.rs)、[前端适配](../../plugins/plugin-ssh/frontend/src/blocks.ts)、[数据契约](../architecture/ssh-plugin.md#命令消息块)、[浏览器验收](../../tests/ssh-blocks-browser.test.mjs)。

本次 Windows 验收：`cargo check --workspace`、`cargo fmt --package coterm-plugin-ssh -- --check`、`cargo clippy -p coterm-plugin-ssh --all-targets -- -D warnings` 通过；SSH Rust 测试 35 项通过，完整 `pnpm test` 57 项通过、0 失败、0 跳过。`pnpm build` 和 `pnpm plugin:ssh:build` 通过，开发包已更新到插件 `dist`。人工复核浅色 / 深色和 320px 窄窗口截图。SSH 端到端契约使用真实插件进程和本地测试服务器，尚未连接用户实际远端；macOS / Linux 未在本轮运行。

## 2026-10-30：使用反馈修复

- 核对发现当前文件工具已经注册，MCP exec 已自动使用 Shell；增加真实 `tools/list` 和工具调用测试，避免把旧插件/客户端缓存现象认定为源码缺失。
- 移除 exec 的单行 / 4096 字节约束；保留共享 PTY 的物理单行限制，错误信息指向 exec 或文件工具。新增 64 KiB 且大量引号的脚本边界测试。
- 修复 ANSI 导致普通彩色输出缺少 `text`，以及带退出码的 exec 摘要遗漏 stdout/stderr。PTY 分包回归覆盖复合命令、目录切换和提示符变化，不把未复现的过滤错位当成已确认根因。
- exec / 文件结果标识执行通道、是否继承 PTY 状态及 sudo。显式提权使用现有宿主秘密交互，密码仅在收到远端 sudo 提示后送入 stdin；免密时不发送密码行，避免破坏文件内容。
- 修复基座秘密交互恢复为异步作业时重复迁移 `Running` 的错误。保持原操作 ID，取消/过期/重连后不重放写操作。
- exec 输出每流每页 4 KiB，缓冲块 4 KiB，避免两条流合并后超过宿主 16 KiB 记录预算；原始字节计数和缺口语义不变。
- 桌面和独立 Stdio 适配器支持 `--addr`，优先于可选环境变量；默认端口仍是 `127.0.0.1:51234`，不需要 Token，参数错误只输出 stderr。

## 2026-10-30：任务列表布局

- 修复历史任务持续换行挤占终端空间：默认单行，右侧展开/收起按钮和总数保持可见；展开内容最多 180px 或窗口高度的 30%，支持垂直滚轮。
- 任务以开始事件顺序从新到旧排列。选中后自动折叠，仅显示最新至选中项的区间，横向滚动至选中项；重新展开可访问更早任务。重复点击选中项保持输出视图，关闭视图仍由独立按钮处理。
- 列表按操作 ID 复用节点，完成事件不会重排；实时新增、状态更新、语言切换和窗口缩放保持选中项或当前浏览锚点。移除选中项清空视图，空列表不占空间；折叠行也支持滚轮浏览。
- 验证：SSH 前端与桌面 `pnpm build` 通过；真实浏览器验证 50 个任务的展开高度、滚轮、键盘选中、实时更新、320px 窄窗口、移除和卸载；终端快捷键相关 13 项测试、桌面布局与主题集成 5 项测试通过。
- 继续入口：[任务 UI](../../plugins/plugin-ssh/frontend/src/index.ts)、[浏览器验收](../../tests/ssh-jobs-browser.test.mjs)。回归命令：`pnpm --filter coterm-plugin-ssh-ui build`，然后 `node --import ./scripts/test-ts-loader.mjs --test tests/ssh-jobs-browser.test.mjs tests/terminal-shortcuts.test.mjs tests/terminal-shortcuts-browser.test.mjs`。

## 验证与入口

- 本轮本地结果：相关 82 项测试通过，工作区 check / Clippy 通过，SSH 开发包构建通过；未连接原反馈的真实远端。
- 回归命令：`cargo test -p coterm-plugin-ssh`、`cargo test -p coterm-mcp`、`cargo test -p coterm-core`。
- 工作区检查：`cargo check --workspace`、`cargo clippy --workspace -- -D warnings`。
- 插件构建：`pnpm plugin:ssh:build`；更新后重启插件/桌面并让 MCP 客户端刷新工具列表。
- 关键实现：[Backend](../../plugins/plugin-ssh/src/backend.rs)、[PTY 边界](../../plugins/plugin-ssh/src/command.rs)、[Session](../../plugins/plugin-ssh/src/session.rs)、[MCP 摘要](../../plugins/plugin-ssh/src/result.rs)。
- 关键验收：[真实插件进程与本地 SSH 服务](../../plugins/plugin-ssh/tests/ssh.rs)、[Stdio 进程](../../crates/coterm-mcp/tests/stdio.rs)。sudo 测试使用真实 POSIX Shell 与测试认证策略，不修改本机管理员权限。
- 契约见 [SSH 架构](../architecture/ssh-plugin.md) 和 [显式提权设计](../design/ssh-explicit-sudo.md)。

## 限制与下次入口

- 消息块只记录获批的 `ssh.command` / `ssh.exec`，原始终端按键不自动分块；MCP commands 批次显示一个块。Agent 必须显式调用 `mcp.ssh.selected_blocks` 读取人工选择的上下文；未经选择时不返回块。真实远程 SSH 交互仍需用户现场复测。
- 尚未连接反馈中的真实远端复测；共享 PTY 的现场空输出问题保留在[反馈](../feedback/ssh_use_feedback.md)，需要原始回放与实际返回值才能进一步归因。
- 独立通道不复制人工 PTY 的 `sudo`/`su`、目录和环境；需要显式 sudo 或在脚本内设置环境。
- 目前仅支持 sudo 单次密码认证和 NOPASSWD；TTY/MFA/自定义交互、Windows 远端 Shell 不在当前提权实现范围。失败或结果未知均不自动重试。
- 保留既有 8 KiB 文件读取、64 KiB 文件写入、120 秒 PTY、30 分钟 exec 和有界输出限制；后续大文件能力应单独设计分页/传输接口。
