# SSH 插件

实现位于 [plugin-ssh](../../plugins/plugin-ssh)。SSH 连接、指纹、认证、PTY、输出与界面均归插件，基座负责加载、实例路由和公共交互。

## 使用

按 [README](../../README.md) 安装依赖后，执行 `pnpm plugin:ssh:build` 和 `pnpm tauri:dev`。

1. 左侧选择 SSH，填写 Host、Port、Username，创建实例。
2. 在实例中 Connect，通过可信来源核对 SHA256 指纹，再输入密码。
3. 用户终端与 MCP 共用该 PTY；可同时创建多个实例，切换界面不关闭连接。
4. SSH 主终端使用左侧 Gutter 块号显示获批命令和回复。Human / Agent 标签在命令前，点击块号多选、高亮；顶部只有「取消全选」。Exec 胶囊可打开右侧或底部详情面板。

MCP 客户端可先用 `coterm.status` 检查桌面与实例连接状态。未连接实例通过 `coterm.list_instances` 获取名称，显式传 `name` 调用 `coterm_ssh_probe` 核对指纹，再用 `coterm_ssh_connect` 提交连接。目标插件仅有一个确认已连接实例且无未知状态时，后续工具可省略 `name`。密码由宿主交互收集，不放入参数；进入审批立即返回 `pending_approval`，批准后用同一 `operationId` 查询结果，不重提。外部名称使用 `coterm_ssh_*`，内部 `ssh.*` / `mcp.ssh.*` 不变，见 [MCP](mcp-integration.md)。

人工终端与串口 Shell 采用同一套[终端快捷键](terminal-shortcuts.md)：Ctrl+C 有选区时复制、无选区时发送中断；Ctrl+F 搜索当前终端输出，Enter/Shift+Enter 跳转、Esc 关闭；Ctrl+V 等组合走本地粘贴，Ctrl+Shift+A 全选终端内容，Esc 优先清除选区；Ctrl+A/E、Ctrl+R 等保留 Shell 语义。焦点在其他输入框时保留输入框原生行为。

## 操作

| 操作 | effect | 用途 |
|---|---|---|
| `ssh.status/probe` | read | 状态、指纹探测；探测不等于信任服务器 |
| `ssh.connect/disconnect` | commit | 认证连接或释放连接 |
| `ssh.reset` | commit | 重建共享 PTY Shell；未完成命令记为 unknown，exec 任务不受影响 |
| `ssh.read` | read | 按游标回放 base64 字节，报告 gap/closed/more |
| `ssh.input` | commit | 人工原始按键，账本隐藏输入参数 |
| `ssh.command` | commit | 按顺序在共享 POSIX PTY 上执行 `commands` 列表并返回退出码 |
| `ssh.exec` | commit | 在独立 exec 通道按顺序执行 `commands` 列表，stdout/stderr 分离，不走共享 PTY |
| `ssh.output` | read | 按游标读取 `ssh.exec` 作业的有界输出与完成状态 |
| `ssh.blocks` | read | 仅人工前端分页读取块快照 |
| `ssh.select_blocks` | draft | 仅 Human 来源设置当前 epoch 下最多 32 个块 ID；空列表清除选择 |
| `ssh.selected_blocks` | read | MCP read Tool，仅返回人工在当前实例选中的块，按 `before` 分页 |
| `ssh.system_info` | read | 固定只读脚本读取用户、内核、发行版等系统信息 |
| `ssh.file_info` | read | 读取单个路径的元信息，不返回内容、不接受任意 shell |
| `ssh.read_file` | read | 读取最多 8 KiB 的 UTF-8 文本文件 |
| `ssh.write_file` | commit | 以受限权限模式原子写入最多 64 KiB 的 UTF-8 文本文件，Agent 默认审批 |
| `ssh.resize` | draft | 调整终端尺寸 |
| `ssh.plugin_info` | read | 插件级信息 |

MCP 显式注册 status、probe、connect、disconnect、reset、command、exec、output、system_info、file_info、read_file、write_file、read，并映射到 `mcp.ssh.*`；不开放原始按键和密码输入。基座可调用的插件级信息操作不代表自动注册了同名 MCP Tool。

两条执行路径的取舍：

- MCP 的 `ssh.command` 和 `ssh.exec` 都接收有序 `commands` 列表（1 至 64 项，总量最多 64 KiB），由插件逐项执行并在首个非零退出码后停止。PTY 每项限单行 4096 字节；exec 每项允许换行、CR、Tab，源脚本受总量 64 KiB UTF-8 字节限制。列表仍作为一个审批快照提交。
- `ssh.command` 保留 `cd`/`export`，适合需要共享会话状态的操作。它仍受 PTY 噪声影响：Shell 作业通知、其他人工输入都可能出现在回放中。
- `ssh.exec` 在单个独立 exec 通道中通过远端 `/bin/sh -c` 按序执行列表，不继承共享 Shell 状态；没有回显、作业通知和登录 Shell 语法错误的连带影响；取消即关闭该通道。长任务优先使用它。
- MCP `ssh.exec` 默认支持管道、重定向、here-document 和多行脚本，不提供也不需要 `useShell` 参数；内部单条 `ssh.exec` operation 仍可用 `useShell` 选择 Shell 模式。插件通过 stdin 传递整个列表脚本，固定 Shell 入口完整接收后再执行，不把脚本塞入 exec 参数。列表项之间保留 `cd`/`export`，但不继承共享 PTY 的目录、环境或提权状态。非交互式检查和文件查看优先用 exec / 文件工具。
- `ssh.system_info`、`ssh.file_info` 使用固定脚本，效果为 read，不进入审批；它们不执行调用方提供的任意命令。`ssh.system_info` 返回 `user/uid/gid/groups/hostname/kernel/arch/shell/home/cwd/os/uptime` 字段，`ssh.file_info` 只返回存在性、类型、大小、权限、属主、mtime 和符号链接目标。
- `ssh.read_file` / `ssh.write_file` 只接收显式远端路径，不提供目录枚举或任意本地文件访问。读写通过独立 SSH exec 通道完成，超时 15 秒；读取最多 8 KiB，拒绝非 UTF-8 文件；写入最多 64 KiB，先写同目录随机临时文件、设置 mode 后原子替换。mode 仅允许 `0600/0640/0644/0700/0750/0755`。写入是 commit，Agent 走基座审批；默认以 SSH 登录用户权限执行。
- 命令由基座「Agent approval」面板按命令文本正则决定放行或进入审批。SSH 目录声明 `x-coterm-approval-text`：内部 command/exec 使用 `/text`，MCP command/exec 使用 `/commands`；文件写入和原始输入不参与命令规则。规则由宿主统一匹配，插件继续负责参数、认证与执行校验，见 [命令审批规则](command-approval.md)。

### 显式 sudo

`ssh.exec`、`ssh.read_file`、`ssh.write_file` 支持可选 `sudo: true`，不支持在 MCP 参数传密码。用户完成基座审批后，桌面通过 `ssh.sudo` 秘密交互收集本次密码；Bypass 只跳过操作审批，不跳过该秘密交互。交互最长 60 秒，绑定实例、操作和物理连接，密码只使用一次，取消、过期或重连后不恢复。

插件通过远端 sudo 提示和子进程就绪标记区分密码认证与 NOPASSWD，只在真正收到密码提示后通过 stdin 发送密码；免密时不发送密码行，文件内容和脚本保持原样。远端拒绝密码、要求 TTY 或额外认证时停止，不自动重试。秘密不进入普通事件、命令字符串、MCP 返回值或持久化。exec / 文件结果的 `executionContext` 明示 `channel=exec`、`inheritsPtyState=false` 和 `sudo`；共享 PTY 明示 `channel=pty`、`inheritsPtyState=true`。方案与验收边界见 [显式提权设计](../design/ssh-explicit-sudo.md)。

## 命令与取消

命令使用随机标记确定输出边界，通过当前 POSIX Shell 的 `eval` 执行，因此 `cd`、`export` 会保留。人工终端通过带可信开始 / 结束标注的回放帧定位块，不从提示符或用户输出猜边界；标注只包含块 ID、编号和来源，命令显示文字按需从人工块快照读取，不放入普通输出事件。Human / Agent 标签仅在前端显示，不发送给 SSH。远端必须处于可接收命令的位置；当前不识别未完成的人工命令或 Windows Shell。

- 执行前先用同一 Shell 的 `-n` 做静态语法检查。语法错误返回 `syntaxError=true` 和退出码 2，不进入 `eval`，共享 PTY 保持可用。检查只解析、不执行；依赖交互别名补全语法的写法可能被拒绝。
- 包装命令的终端回显在进入回放前被移除；提示符和真实输出保留。远端关闭回显时不需要额外处理。
- 每连接同时只跟踪一个命令，最长 120 秒；与客户端等待上限分别计时。`ssh.exec` 的作业上限为 30 分钟，最多 8 个并行作业。
- 源命令最多 4096 字节，转义包装后还受 4000 字节 PTY 单行上限约束。
- 结果字段：`exitCode`、`encoding`、`text` 或 `data`、`bytes`、`originalBytes`、`droppedBytes`、`limit`、`truncated`、`syntaxError`、`durationMs`。UTF-8 终端文本返回 `text`，保留 ANSI、换行和常见终端控制字符；NUL、其他二进制控制字节、无效/截断 UTF-8，或 JSON 转义后超过结果预算时返回无损 base64 `data`。捕获仍限 8 KiB；字节统计按原始字节计算。非零退出的 MCP 结果 `isError=true`。
- 取消先发送 Ctrl+C，最多等待 2 秒确认结束标记。确认成功为 `stopped`；否则返回 `unknown` 并自动重建 PTY Shell，连接保持可用。可用 `ssh.reset` 主动重建。
- 断线、结束标记缺失或跟踪到期也可能 unknown；不推测成功、不自动重发。跟踪到期会自动重建共享 PTY，并发送 `ssh.status(reset=true)`；如果重建失败则连接关闭，需要重连。`ssh.exec` 作业不受此限制。

### exec 输出与进度

`ssh.exec` 的结果包含 `exitCode`、`signal`、`timedOut`、`cancelled`，以及每流最多 32 KiB 的 `stdout`/`stderr` 预览。MCP 文本摘要包含两条流的可读预览，每流按 JSON 转义成本最多 34 KiB、整体最多 68 KiB；日常 32 KiB 普通文本无需额外分页。机器可读副本始终在 `structuredContent`，不可读字节或 JSON 扩张过大时无损返回 base64，普通非 exec 摘要仍限 2 KiB。

流保存在有界作业缓冲中（每流 64 KiB、最多 8 个作业），超出保留容量会淘汰旧块，不承诺保存全部历史。通过 `coterm_ssh_output` 用 `stdoutCursor`/`stderrCursor` 读取，每流每次最多 32 KiB，返回 `done`、`exitCode`、`gap`、`more`。超过预览的输出仍需分页，遇到 `gap` 必须向使用者说明缺口。宿主输入记录预算保持 16 KiB，输出独立为 192 KiB，容纳两流、文本摘要和 JSON/base64 扩张；MCP 单帧仍限 1 MiB。取消会关闭通道；通道关闭确认后返回 `stopped`，否则 `unknown`。作业缓冲随连接销毁。

## 输出与限制

每连接回放缓冲 256 KiB，每块最多 4 KiB；普通 `ssh.read` 单次最多 8 KiB，终端帧模式每页一个有界帧。开始 / 结束帧由插件生成，包含 ID、编号、来源和 `boundary=verified/observed`；人工观察的开始帧另含 `lineOffset`，不含命令、输出或秘密。事件为 `ssh.terminal.output`、`ssh.status` 和 `ssh.job`，携带连接 ID 和游标。`ssh.job` 仅含 `operationId`、源命令和 `done`，不携带输出；详情通过 `ssh.output` 按游标读取。`ssh.status` 重建时带 `reset=true`。前端先订阅再回放，缺口时恢复；输入队列上限 16 KiB，控制队列 64 项，终端滚动缓存 3000 行。

Exec 胶囊保留单行列表。用户查看历史任务后暂停跟随；点最新任务、恢复按钮或在面板焦点内按 `Esc` 恢复。仅已打开且跟随最新的面板随新任务更新；关闭面板时新任务不会自动展开。支持右侧 / 底部停靠、拖拽分隔条和按实例保存尺寸。

Exec 工具栏只保留名称、必要的运行 / 失败状态、停靠及关闭图标；成功状态由任务胶囊表示，不重复显示「已成功」。完整命令位于工具栏下方的可滚动正文中，支持多行 / 自动折行，与输出共用终端字号。STDOUT 和 STDERR 各有独立边框与文字标签，分别使用主题蓝色和错误色；没有内容的流隐藏。关闭图标保留本地化 tooltip、可访问名称及键盘操作，不终止任务。

主终端的 flex / 裁切样式只作用于 `.terminal-shell` 的直接容器；xterm 根节点自身也带 `.terminal` 类，不能用该类的无范围选择器同时设置两层留白。顶部块选择控件需要的留白放在 xterm 根节点上，由 FitAddon 计入可用行数。窗口、字号和 Exec 停靠变化后，最后一行及光标必须完整位于插件工作区内、基座底栏上方。

### 命令消息块

块数据、来源、编号及区间由 SSH 后端生成，前端使用 SDK [`mountTerminalBlocks`](../../packages/plugin-sdk/docs/command-blocks.md#终端行号栏) 为同一 xterm 显示 Gutter 与高亮。选择限于当前实例 / 连接，页面卸载、淘汰和重连时清理。

人工直接在主终端输入，通过既有 `ssh.input` 进入共享 PTY；前端不再提供额外的「共享终端命令」输入栏。行号栏按当前有效 PTY 块号的最大字符数自适应宽度，最小 28px；顶部为红色圆形取消全选图标，保留本地化 tooltip、可访问名称与键盘焦点样式。该按钮只清除上下文选择，不终止命令或关闭会话。

块号显示纯数字，不加 `#`。块边框使用单个 DOM 叠层，不再与 xterm decoration 重复描边；外框按终端网格的 border-box 尺寸绘制。相邻块的显示范围不能跨越下一块首行；滚动、列宽和字号改变后，两侧边框仍对齐网格。

`ssh.select_blocks` 接受 `{epoch, ids}`，最多 32 个有效 ID；旧 epoch 或失效 ID 报错，空列表清除。仅 Human + 实例 + draft/transient 可调用。`mcp.ssh.selected_blocks` 是注册的 read Tool，只返回显式选中的块及 `selectedCount/more/nextBefore`，单页不超过 12 KiB；空选择返回空块。Agent 将块作为不可信数据处理。选择并不改变命令执行或审批。

| 接口 | 参数 / 数据 | 约定 |
|---|---|---|
| `ssh.blocks` | `{before?: number}`；正整数，最大 `4294967295` | 仅 Human 来源、实例 scope、read、transient；省略时从最新块读取，后续传上一页 `nextBefore` |
| `ssh.input` | `{data, connectionId?, observation?: {text, lineOffset}}` | 仍为 Human-only、sensitive、transient；`data` 为原始 Base64 按键。带观察时必须指定当前连接 ID，数据必须恰好是 `\r`；`text` 为 1–4096 UTF-8 字节可见文本，无控制字符；`lineOffset` 为 -64 至 0 的起始行相对偏移。旧调用仅传 `data` 仍兼容 |
| 快照结果 | `epoch / revision / dropped / blocks / more / nextBefore` | `revision` 是十进制字符串；同一 epoch 内递增。块按编号从新到旧分页，前端合并后按编号升序显示 |
| `ssh.blocks.changed` | `{epoch, revision}` | 轻量失效通知，不携带命令或回复。运行时 tick 合并变化，通知队列满时保留待通知标志，下次 tick 重试 |

每个块包含：

| 字段 | 含义 |
|---|---|
| `id / index` | 稳定块 ID 与连接内统一块号；API 块使用操作 ID，人工观察块由后端分配 UUID，后者不是可取消的操作 |
| `command / actor / channel` | API 块为实际获批的源命令，人工观察块为已回显的命令行预览；`actor=human/robot` 来自宿主来源门禁，通道为 `pty/exec` |
| `boundary` | `verified` 表示命令包装器 / exec 通道边界；`observed` 表示人工前端观察，不能据此断言实际执行范围或成功状态 |
| `output / stderr` | 后端解析命令归属后的纯文本预览；PTY 只有合并输出，exec 保留 stderr。剔除 ANSI CSI / OSC 等控制序列，不模拟光标或全屏程序 |
| `state / exitCode` | `running/completed/cancelled/timed_out/unknown`；有远端退出码才填写 `exitCode`，非零完成也仍是 completed 并显示失败标记 |
| `commandTruncated / outputTruncated` | 命令或回复预览是否截断，前端明确提示 |

- 每个 `ssh.command` / `ssh.exec` 操作形成一个块；MCP `commands` 批次按一个获批操作显示，源命令用换行连接，不伪造批次内逐项输出边界。仅执行阶段登记块，准备审批、拒绝或秘密交互取消不会创建已执行块。
- API 操作使用随机 PTY 边界和独立 exec 通道归属回复。人工 Enter 观察另见下节；原始 `ssh.input.data` 从不作为块文本保存，宿主秘密句柄和值不进入块。
- 每连接最多保留 32 个块，优先移除最早的已终结块，保留运行中的块；`dropped` 记录淘汰数量。每块命令预览最多 1 KiB UTF-8，stdout / stderr 合计保留前 2 KiB 原始字节。原始字节经有损纯文本展示，完整终端回放与 `ssh.output` 仍保留原有上限。
- 消息块单页 JSON 仍按实际序列化结果限制在 12 KiB 内，不随 exec 预算扩大；预览限制与序列化规则在 Windows / macOS / Linux 一致，没有新增平台进程、权限或依赖。
- 记录只在插件内存中保存。关闭前端后可重新读取；断开连接仍可查看最近快照，成功重连使用新的 epoch 并清空旧历史 / 选择。重置共享 Shell 保留当前连接历史，将未确认命令标为 unknown。
- 前端先安装实例事件订阅，再在打开消息块视图时读快照；仅当前可见视图响应失效通知，并以 100ms 合并读取。分页中 epoch / revision 改变时重新读取；事件缺口触发恢复。卸载注销订阅、取消计时器，晚到结果不再操作 DOM。

#### 人工 Enter 观察

前端在普通 buffer 的常见 Shell 提示符（以 `$ `、`# `、`% ` 结束）后首次输入时保存 marker 和起始列；Enter 前读取 xterm 中已经回显的可见行，支持历史召回、行内编辑和最多 64 行软折行。回显尚未到达时最多等待 180ms；读取的是可见预览，不积累密码敏感的按键历史。空行、无回显输入、备用屏幕、批量多行粘贴和中断输入不创建块。自定义提示符、回显延迟超过等待上限、复杂多行编辑不能保证识别，应使用 API 操作获取可靠边界。

前端把观察附在原始 Enter 的 `ssh.input` 上，后端先分配统一块号、发出 `boundary=observed` 开始帧，再原样发送 Enter；不会额外执行 `ssh.command`、向远端注入标记或修改 Shell 配置。SDK 以相对行偏移将块号锚定到命令首行。此时命令已存在于终端，前端不插入来源标签打乱 readline 光标，Human 来源可从块号可访问名称及块快照读取。观察帧和 API 帧均可回放重建块号。

人工块的视觉范围在识别到返回的同类提示符后截止于提示符前一行，不把下一次输入区域包进上一块。识别基于当前已解析的 xterm buffer，支持分包提示符和 `user@host:目录` 的目录变化；marker 随软折行重排。此处仅裁剪前端显示，不把提示符认定为真实执行完成，不改变后端预览 / `boundary` / 状态或退出码。自定义提示符可能不能识别；下一块首行仍提供视觉上限。

每连接最多一个进行中的人工观察，保留后续 PTY 输出的有界预览；下一次原始输入、API PTY 命令、重置或断线结束它。结束前状态为 `running`，结束后为 `unknown`，退出码始终为 null；这只是观察窗口，不是 Shell 完成状态，输出可能包含提示符 / 异步通知。新原始输入在写入前关闭观察，避免交互输入的回显被归入上一条命令。人工块与 API 块共用原有快照、淘汰和显式选择接口，Agent 读取时必须保留 `boundary` 的不确定性。

连接错误带稳定阶段：`tcp`、`handshake`、`host_key`、`auth`、`channel`、`pty`、`shell`、`connect`。指纹不匹配使用 `ssh_host_key_mismatch`；远端 Shell 退出使用 `ssh_remote_shell_exited` 并附退出码，网络中断使用 `ssh_connection_lost_before_completion`。

指纹每次连接确认，不持久化。尚无私钥、ssh-agent、MFA、SFTP、端口转发、自动重连和凭据保存；消息块通过 `mcp.ssh.selected_blocks` 向 Agent 提供人工明确选择的上下文。秘密和背压规则见 [运行时通信](runtime-communication.md)。

## 验证

`cargo test -p coterm-plugin-ssh` 使用本地测试 SSH 服务，覆盖指纹/密码失败、多实例、真实插件进程、审批、秘密隔离、共享 PTY、输出缺口和 MCP。远程服务器及真实桌面验收需单独执行，最近验证范围见 [SSH 开发进度](../progress/ssh-plugin.md)。

块界面回归：先执行 `pnpm --filter coterm-plugin-ssh-ui build`，再运行 `node --import ./scripts/test-ts-loader.mjs --test tests/ssh-blocks-browser.test.mjs tests/ssh-human-blocks-browser.test.mjs tests/ssh-jobs-browser.test.mjs`。真实 SSH 进程契约包含人工 Enter 观察与 MCP 选择、审批编辑、可信来源、原始按键隔离、事件、断开保留和重连恢复；内存日志测试覆盖 UTF-8 / ANSI 分片、截断、分页预算及慢消费者。
