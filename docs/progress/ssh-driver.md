# SSH 驱动开发进度

## 当前能力

- 多实例 SSH、人机共用 PTY；统一 command 提交入口，默认前台 terminal，显式 exec 用于预计长期运行或用户要求的后台任务。
- MCP 对外使用 `coterm_ssh_*`；以实例 `name` 路由，唯一确认已连接且无未知状态时可省略；文件读写作为显式实例工具发布。
- `ssh.command` 的两种模式均接受多行脚本，源文本合计 64 KiB UTF-8；列表项共享各自执行 Shell 的目录/环境，首个非零状态停止，exec 不继承交互 PTY 状态。
- `ssh.command(mode=exec)`、`ssh.read_file`、`ssh.write_file` 支持显式 `sudo: true`，审批后由桌面收集一次性秘密；不接受 Agent 传密码。
- ANSI 终端文本保留控制字符并直接返回 UTF-8，二进制、无效 UTF-8 或控制字符过密时无损返回 Base64；摘要同时呈现 stdout/stderr，限制 JSON 扩张。
- 后台任务默认单行，从新到旧排列；可展开限高列表，选中后自动收起并将选中项定位到行末。
- 主终端使用 SDK xterm Gutter 显示 PTY 命令块号和选中高亮；Human / Agent 标签在命令前，按钮仅有取消全选。人工直接在主终端输入，不另设底部命令栏。
- Exec 胶囊有 LED 状态、右侧 / 底部停靠、可拖动尺寸；历史选择暂停跟随，关闭面板时新任务不自动展开。
- Agent 使用 read_lines、read_selected_lines、read_dialogue_pairs 读取后端快照，无需维护游标。xterm 文本选区由仅 Human 可用的 select_lines 同步；块号选择仍用于前端高亮。

## 2026-11-11：进程内运行

- SSH Backend 作为 Rust 库由桌面注册，不再启动独立驱动程序。保留连接对象、会话 actor、命令块、后台通道和秘密交互；停止时依靠连接析构取消任务。
- 宿主/MCP 集成测试改为注册真实 Backend 与本地 SSH 测试服务器，保留审批、认证、前后台命令、文件与 sudo 测试，不再复制 EXE 或组装测试包。
- 工作区测试与严格 Clippy 通过；本轮未连接外部真实 SSH 服务器。整体迁移及故障隔离边界见[项目进度](project.md)。

## 2026-11-08：统一命令与后端快照

- command 的 mode 默认为 terminal；显式 exec 复用独立后台通道和秘密交互。两种路径复用审批，mode 和命令一起保留在参数快照中。MCP 不再发布独立 exec、output、read 和 selected_blocks 工具。
- PTY 支持分片交付长脚本和多行脚本，完整 brace group 解析后执行并保留 Shell 状态；删除运行 120 秒后自动重置终端的行为。等待超时不迁移、不重新执行命令。
- 后端命令记录统一包含 stdin/stdout/stderr、mode、status、streamMode、exitCode、truncated 和 error；空流保留空字符串，未知值保留 null。ExtendedData 单独保存且不注入主 xterm，远端已合并的内容仍作为普通输出。
- 每实例当前连接最多 32 条记录，源输入最多 64 KiB、每流保留最新 64 KiB；单记录及批量读取按实际 JSON 限制在 96 KiB。删除 JobRegistry 的重复输出缓冲，保留任务容量与取消管理。
- read_lines 默认读取终端最近逻辑行，也可按 operationId 查询命令；read_dialogue_pairs 同时覆盖前台和后台，运行中数据可读。断线保留后端历史，成功重连清理。JSON 仅为内存记录及传输格式，没有新增落盘结构。
- xterm 选区同步绑定物理连接，Agent 不能写选择；前端卸载、选区清除及断线清理。读取单次 16 KiB，截断可见。终端回放序号仍为前端内部恢复机制，不暴露给 Agent。
- 验证：SSH 31 项库测试及 23 项集成测试通过，覆盖真实子进程、默认 PTY、显式 exec、审批及 sudo、32 KiB 结果、长脚本、分流、运行中读取、跨实例隔离、选区来源门禁和断线保留。仅将本次暂存源码独立导出后，54 项 Rust 测试再次通过。工作区 cargo check、SSH Clippy、SSH 前端及桌面构建通过；3 项真实浏览器回归通过、无跳过，覆盖 xterm 选区同步/清除、命令块及后台快照显示；文档链接、docs 根目录和保留 CRLF 的空白检查通过。
- 下次入口：[SSH 操作入口](../../crates/coterm-driver/src/ssh/backend.rs)、[命令记录](../../crates/coterm-driver/src/ssh/blocks.rs)、[终端交付与边界](../../crates/coterm-driver/src/ssh/command.rs)、[架构契约](../architecture/ssh-driver.md)。本次 SSH 命令阶段未改动审批存储或串口接口。每实例审批对象/配置迁移已由后续宿主阶段接入，见 [MCP 进度](mcp.md#2026-11-08每实例审批对象与独立配置)；串口 Shell/stream 已完成，见 [串口进度](../../crates/coterm-driver/docs/serial/progress.md)。

## 2026-11-03：现场块边界与 Exec 展示反馈

- 人工观察块在常见提示符返回时，将视觉结束 marker 放在提示符前一行；兼容提示符分包和 `cd /` 后目录变化，避免下一条提示符进入上一块。该判断只裁剪前端边框，不改变后端观察窗口、输出预览、unknown 状态或空退出码；自定义提示符仍可能无法识别。
- SDK 移除重复的 xterm decoration 描边，保留单层 border-box 边框；相邻块以后一块首行为上限。块号改为纯数字，滚动、软折行、列宽与字号变化时边框保持在终端网格内。
- Exec 命令移至工具栏下方正文，与输出共用终端字号并支持多行折行；成功不再重复显示「已成功」，失败仍显示退出码。STDOUT / STDERR 使用独立的主题蓝色 / 错误色边框，空流隐藏，关闭改为带本地化 tooltip 的图标。
- 定向浏览器回归 8 项通过、0 失败、0 跳过，覆盖分包提示符、无回复命令、选择 / 回放、重排对齐、双输出流、双语主题、大字号窄屏、关闭 / 跟随、快捷键与审批界面。桌面和 SSH 前端构建通过；截图在忽略目录 `target/ssh-feedback-20261103/` 完成人工复核。
- 完整 `pnpm driver:ssh:build` 的 Rust 编译与前端构建通过，但复制后端 exe 时遇到运行中驱动占用（EBUSY），未停止用户会话；前端产物已更新。退出旧桌面后可重新完整打包，本次无需后端改动。
- 另将暂存源码独立导出，单独通过桌面 / SSH 的 TypeScript 与 Vite 构建，以及审批 / SSH 块 / 人工块 / Exec 共 4 项浏览器回归，确认本次提交不依赖工作区其他未提交修改。
- 本次未修改 Rust、协议、依赖或权限，尚未连接用户真实远端。入口：[视觉边界观察](../../crates/coterm-driver/frontend/ssh/src/observed-boundaries.ts)、[SDK 边框](../../apps/shared/components/terminal-blocks.ts)、[Exec UI](../../crates/coterm-driver/frontend/ssh/src/jobs.ts)、[交互约束](../architecture/ssh-driver.md#命令消息块)。

## 2026-11-03：人工终端 Enter 生成可选命令块

- 复现真实 xterm 人工输入 `ls` / Enter 后 Gutter 仍为空：原始输入只有 PTY 透传，未经过 API 命令包装器，因此没有开始帧。新增前端可见行观察，在常见 `$ ` / `# ` / `% ` 提示符后回车时随 `ssh.input` 发送命令预览；后端统一分配编号、输出观察帧并保留回复，按键仍原样发送，不重复执行或修改远端 Shell。
- 人工块标记 `boundary=observed`，与 API 的 `verified` 明确区分；退出码保持 null，下一次原始输入 / PTY API 操作 / 重置 / 断线结束观察后为 unknown。原始按键与秘密不作为命令保存；观察和选择共用现有 32 块、1 KiB 命令、2 KiB 回复及 12 KiB 单页预算。
- 支持历史召回、退格 / 行内编辑、软折行、单行粘贴、短暂延迟回显和重新挂载回放；空回车、无回显密码、备用屏幕和批量多行粘贴不生成块。连接 ID 门禁拒绝旧前端输入，resize / 缺口清理未提交观察，API 边界期间暂停人工观察。
- 另复现并修复快照竞态：开始帧到达后，较早的快照不能删除新 marker；已失效的进行中读取丢弃并重读。浏览器用例通过人为延迟旧空快照，先复现块号丢失再验证修复。
- 验证：44 项 SSH Rust 测试通过（24 单元、20 真实驱动 / 本地 SSH 集成），覆盖人工 Enter、Human-only 门禁、旧连接拒绝、原始按键隔离、选择后 MCP 读取与边界回放；工作区 check、SSH all-targets Clippy、桌面 build 和 SSH 开发包构建通过。浏览器覆盖人工输入到块选择、历史 / 编辑 / 粘贴 / 折行、延迟回显、密码与备用屏幕隔离、挂载恢复及原有 API / Exec / 快捷键。
- Exec 浏览器原有滚动断言在改动前基线同样出现时序失败，现等待语言切换后的异步布局完成再检查，没有放宽滚动位置条件。
- 入口：[人工输入观察](../../crates/coterm-driver/frontend/ssh/src/human-commands.ts)、[会话登记](../../crates/coterm-driver/src/ssh/session.rs)、[浏览器回归](../../tests/ssh-human-blocks-browser.test.mjs)、[接口与限制](../architecture/ssh-driver.md#人工-enter-观察)。重启 SSH 驱动 / 桌面加载新前后端；本轮使用本地测试服务器，尚未连接用户真实远端。

## 2026-11-02：主终端底部裁切修复

- 浏览器回归复现终端画布比自身容器底部多出约 24px。根因是 xterm 根节点也带 `.terminal` 类，无范围样式给外层容器与内层终端重复添加顶部留白，而 FitAddon 只扣除内层留白。
- 将容器 flex / overflow 样式限定为 `.terminal-shell` 的直接子元素，留白仅作用于 xterm 根节点，行数计算与实际可用高度一致。
- 新增真实桌面 + SSH bundle + 基座 footer 回归：连续写入 100 行后，确认最后一行提示符完整留在终端容器及基座底栏上方。覆盖 1280×800 / 13px、1024×600 / 20px、768×480 / 16px；Exec 回归另检查右侧 / 底部停靠、拖拽及 320px 窄窗口。
- 本轮 5 项浏览器集成测试通过，包含底部边界、明暗主题、语言与安装状态、命令块选择、Exec 任务交互。`pnpm build` 和 SSH 前端构建通过，`dist/frontend/index.js` 已更新；完整 `pnpm driver:ssh:build` 在复制后端 exe 时因运行中的驱动占用而报 EBUSY，未停止用户会话。未修改 Rust、宿主权限或协议，退出旧桌面后可重跑完整打包。
- 回归入口：`node --import ./scripts/test-ts-loader.mjs --test --test-concurrency=1 --test-name-pattern="SSH|appearance switches live" tests/browser-layout.test.mjs tests/ssh-blocks-browser.test.mjs tests/ssh-jobs-browser.test.mjs`。截图保存在 `target/ssh-bottom-layout-*.png`，不覆盖既有布局截图；原问题在修复前测试失败、修复后通过。

## 2026-11-02：Agent MCP 使用反馈

- 修复审批导致首次 MCP 调用等待 60 秒：SSH descriptor 声明 `returnOnApproval`，进入审批时即时返回 `pending_approval` 和原操作 ID。集成测试改为批准后查询同一操作，继续覆盖并发状态查询、秘密输入、取消和无重放。
- exec 完成预览 / `ssh.output` 每流每页由 4 KiB 增至 32 KiB。普通文本摘要可保留两条流，整体按 JSON 成本限 68 KiB；两流和结构化内容容纳于宿主独立的 192 KiB 输出预算。每流保留 64 KiB、最多 8 作业、输入预算、PTY / 消息块限额不变。
- 状态工具声明连接知识，接入 `coterm.status`；单个确认已连接 SSH 可直接省略 `name`，多个连接或未知时拒绝默认提交。已断开 / 未连接的实例仍可通过显式名称查询和连接。
- 对外工具名扁平化为 `coterm_ssh_exec` 等，旧名保留兼容；驱动内部 `ssh.*` / `mcp.ssh.*` 不变。
- 回归先复现旧 4 KiB 截断和审批不在 500ms 内返回，再验证修复。本地真实 POSIX Shell + MCP 连接验证 32 KiB 输出完整出现在首次结果、可读文本和操作查询；另测 ANSI、UTF-8 / 二进制两流预算、默认目标、多连接拒绝和旧名称。
- 本轮 SSH Rust 42 项通过（23 单元 + 19 进程集成），工作区 check、相关 all-targets Clippy 和 SSH / Serial 开发包构建通过。Core 存在与当前 Local manifest 对应的 5 项安装 fixture 失败，完整说明见 [MCP 进度](mcp.md#2026-11-02agent-健康检查默认路由与扁平工具名)。尚未连接用户实际远端或运行 macOS / Linux 实机验收。
- 决策：[Agent 调用体验](../architecture/mcp-integration.md)；稳定接口：[MCP](../architecture/mcp-integration.md)、[exec 输出](../architecture/ssh-driver.md#后端-json-记录与读取)。重启桌面及 MCP 连接后使用新实现。

## 2026-11-02：主终端块选择与 Exec 面板

- 根据现场截图收紧界面：取消全选改成 16px 红色圆形叉号，保留本地化 tooltip / 可访问名称和禁用状态；块号栏从固定 66px 改为按有效 PTY 块号位数伸缩，最小 28px。移除底部共享命令表单及无内容的历史占位，原有 `ssh.input`、可信块边界和 Agent 操作接口保持一致。
- 本次界面回归：SSH 开发包、桌面前端与 SDK 浏览器构建通过；全量前端测试串行运行 62 项通过、无跳过。真实浏览器覆盖红色圆形控件、最大块号不截断、行号栏伸缩、直接终端输入 / Enter、取消全选、明暗主题及 320px 宽度。任务面板缩放断言等待响应式底部停靠完成，避免把过渡中的零宽列表当成稳定布局；测试前后校验并恢复原有四张布局截图。
- 命令起止通过后端可信边界转换为有界 PTY 帧元数据（仅 ID、编号、来源，不带命令或输出），SDK 使用 xterm marker 和 decoration 定位块号；命令显示文字按需从人工块快照读取。UI 标签只写入本地终端显示，不改变 SSH 输入字节。备用屏幕隐藏 PTY 标记，scrollback 淘汰时清理 ID 与选择。
- 块选择只通过实例 / epoch 绑定的 `ssh.select_blocks` 更新；Human MCP Tool `mcp.ssh.selected_blocks` 只读所选不可信数据、明确截断状态和分页信息。重连及卸载清空选择；历史淘汰自动修剪选择。
- Exec 新任务只在详情面板打开且跟随最新时切换；已关闭面板不被自动展开。历史提示、局部 Esc、方向停靠、Splitter 拖动和按实例尺寸保存已接通。
- 文档：[产品约束与验收](../architecture/ssh-driver.md#命令消息块)、[SSH 架构契约](../architecture/ssh-driver.md)、[SDK 终端行号栏](../../apps/shared/docs/command-blocks.md#终端行号栏)。
- 验收：SSH 前端真实浏览器覆盖边界、Human/Agent 标签、实例选择上下文、按键焦点、备用屏幕、scrollback 淘汰、主题、320px 布局；Exec 浏览器覆盖关闭面板、跟随/锁定/恢复、方向停靠、拖动与卸载。真实 SSH 测试覆盖 Human 写选择、Agent 只读、stale epoch 拒绝和原始按键不入上下文。

## 2026-11-01：消息块首次接入

- 后端新增内存块日志和仅人工前端使用的 `ssh.blocks` 分页操作；沿用现有 PTY 随机标记与 exec 通道归属回复，不记录原始按键或秘密交付。
- 块保留真实操作 ID、获批后的命令、可信来源、通道、状态和退出码。未确认结束 / 重置 / 断线标为 unknown，取消和超时单独显示；MCP 列表按一次操作形成一个块。
- 每连接保留最多 32 块，命令预览 1 KiB、stdout / stderr 合计预览 2 KiB，单页 JSON 小于 12 KiB；淘汰与截断可见。事件只携带 epoch / revision，在 tick 合并并处理慢消费者。
- SSH 前端直接导入 SDK `mountBlockList`，保留完整终端、原任务行及输出视图；消息块新增共享 PTY 单行输入，焦点、文本选择、块选择在回复更新时保持。只在块视图可见时读取，错误可重试，缺口重读，成功重连清理旧 epoch。
- 无新增依赖、宿主接口、跨平台启动方式或权限；所选上下文提交给 Agent 仍待后续接入。

实现入口：[后端日志](../../crates/coterm-driver/src/ssh/blocks.rs)、[前端适配](../../crates/coterm-driver/frontend/ssh/src/blocks.ts)、[数据契约](../architecture/ssh-driver.md#命令消息块)、[浏览器验收](../../tests/ssh-blocks-browser.test.mjs)。

本次 Windows 验收：`cargo check --workspace`、`cargo fmt --package coterm-driver-ssh -- --check`、`cargo clippy -p coterm-driver --all-targets -- -D warnings` 通过；SSH Rust 测试 35 项通过，完整 `pnpm test` 57 项通过、0 失败、0 跳过。`pnpm build` 和 `pnpm driver:ssh:build` 通过，开发包已更新到驱动 `dist`。人工复核浅色 / 深色和 320px 窄窗口截图。SSH 端到端契约使用真实驱动进程和本地测试服务器，尚未连接用户实际远端；macOS / Linux 未在本轮运行。

## 2026-10-30：使用反馈修复

- 核对发现当前文件工具已经注册，MCP exec 已自动使用 Shell；增加真实 `tools/list` 和工具调用测试，避免把旧驱动/客户端缓存现象认定为源码缺失。
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
- 继续入口：[任务 UI](../../crates/coterm-driver/frontend/ssh/src/index.ts)、[浏览器验收](../../tests/ssh-jobs-browser.test.mjs)。回归命令：`pnpm --filter coterm-driver-ssh-ui build`，然后 `node --import ./scripts/test-ts-loader.mjs --test tests/ssh-jobs-browser.test.mjs tests/terminal-shortcuts.test.mjs tests/terminal-shortcuts-browser.test.mjs`。

## 验证与入口

- 本轮本地结果：相关 82 项测试通过，工作区 check / Clippy 通过，SSH 开发包构建通过；未连接原反馈的真实远端。
- 回归命令：`cargo test -p coterm-driver`、`cargo test -p coterm-mcp`、`cargo test -p coterm-core`。
- 工作区检查：`cargo check --workspace`、`cargo clippy --workspace -- -D warnings`。
- 驱动构建：`pnpm driver:ssh:build`；更新后重启驱动/桌面并让 MCP 客户端刷新工具列表。
- 关键实现：[Backend](../../crates/coterm-driver/src/ssh/backend.rs)、[PTY 边界](../../crates/coterm-driver/src/ssh/command.rs)、[Session](../../crates/coterm-driver/src/ssh/session.rs)、[MCP 摘要](../../crates/coterm-driver/src/ssh/result.rs)。
- 关键验收：[真实驱动进程与本地 SSH 服务](../../crates/coterm-driver/tests/ssh.rs)、[Stdio 进程](../../crates/coterm-mcp/tests/stdio.rs)。sudo 测试使用真实 POSIX Shell 与测试认证策略，不修改本机管理员权限。
- 契约见 [SSH 架构](../architecture/ssh-driver.md) 和 [显式提权设计](../architecture/ssh-driver.md#显式-sudo)。

## 限制与下次入口

- 消息块记录 API 操作和常见 Shell 下人工 Enter 的可见行观察；自定义提示符、复杂多行编辑、批量多行粘贴或超过 180ms 的回显延迟不能保证识别。人工观察不是可信完成边界，状态 / 退出码不能当作执行结果；需要可靠边界时使用 `ssh.command`。MCP commands 批次显示一个块。Agent 使用 `read_selected_lines` 读取人工选择的文本，无选择时返回空文本；`read_dialogue_pairs` 可读取最近命令记录。真实远程 SSH 交互仍需用户现场复测。
- 尚未连接反馈中的真实远端复测；共享 PTY 的现场空输出问题保留在[反馈](../feedback/ssh_use_feedback.md)，需要原始回放与实际返回值才能进一步归因。
- 独立通道不复制人工 PTY 的 `sudo`/`su`、目录和环境；需要显式 sudo 或在脚本内设置环境。
- 目前仅支持 sudo 单次密码认证和 NOPASSWD；TTY/MFA/自定义交互、Windows 远端 Shell 不在当前提权实现范围。失败或结果未知均不自动重试。
- 保留既有 8 KiB 文件读取、64 KiB 文件写入、30 分钟 exec 和有界输出限制；PTY 不再因运行超过 120 秒自动终止。后续大文件能力应单独设计分页/传输接口。

## 2026-10-27：早期现场回归摘要

真实 Tauri / MCP / 已认证远端验证过语法错误后连接可用、非零退出码、输出截断元数据、结构化结果、取消后恢复和包装标记过滤。该结果对应当时版本，不代表当前重构已在原远端复测；远端网络中断、真实指纹 / 密码失败、跨公网长连接与 exec 并发仍需现场验证。剩余空输出问题保留于 [SSH 反馈](../feedback/ssh_use_feedback.md)。
