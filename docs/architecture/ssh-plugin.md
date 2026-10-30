# SSH 插件

实现位于 [plugin-ssh](../../plugins/plugin-ssh)。SSH 连接、指纹、认证、PTY、输出与界面均归插件，基座负责加载、实例路由和公共交互。

## 使用

按 [README](../../README.md) 安装依赖后，执行 `pnpm plugin:ssh:build` 和 `pnpm tauri:dev`。

1. 左侧选择 SSH，填写 Host、Port、Username，创建实例。
2. 在实例中 Connect，通过可信来源核对 SHA256 指纹，再输入密码。
3. 用户终端与 MCP 共用该 PTY；可同时创建多个实例，切换界面不关闭连接。

MCP 客户端先用 `coterm.list_instances` 定位实例，再 `ssh.probe` 获取并核对指纹、`ssh.connect` 提交连接。密码由宿主交互收集，不放入工具参数；审批/密码等待后返回原操作结果。外部工具名称带插件命名空间，见 [MCP](mcp-integration.md)。

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
- 需要放行其他命令时，由基座的「Agent approval」面板按操作名显式添加；不要在插件里依据命令字符串判断安全性。

### 显式 sudo

`ssh.exec`、`ssh.read_file`、`ssh.write_file` 支持可选 `sudo: true`，不支持在 MCP 参数传密码。用户完成基座审批后，桌面通过 `ssh.sudo` 秘密交互收集本次密码；Bypass 只跳过操作审批，不跳过该秘密交互。交互最长 60 秒，绑定实例、操作和物理连接，密码只使用一次，取消、过期或重连后不恢复。

插件通过远端 sudo 提示和子进程就绪标记区分密码认证与 NOPASSWD，只在真正收到密码提示后通过 stdin 发送密码；免密时不发送密码行，文件内容和脚本保持原样。远端拒绝密码、要求 TTY 或额外认证时停止，不自动重试。秘密不进入普通事件、命令字符串、MCP 返回值或持久化。exec / 文件结果的 `executionContext` 明示 `channel=exec`、`inheritsPtyState=false` 和 `sudo`；共享 PTY 明示 `channel=pty`、`inheritsPtyState=true`。方案与验收边界见 [显式提权设计](../design/ssh-explicit-sudo.md)。

## 命令与取消

命令使用随机标记确定输出边界，通过当前 POSIX Shell 的 `eval` 执行，因此 `cd`、`export` 会保留。远端必须处于可接收命令的位置；当前不识别未完成的人工命令或 Windows Shell。

- 执行前先用同一 Shell 的 `-n` 做静态语法检查。语法错误返回 `syntaxError=true` 和退出码 2，不进入 `eval`，共享 PTY 保持可用。检查只解析、不执行；依赖交互别名补全语法的写法可能被拒绝。
- 包装命令的终端回显在进入回放前被移除；提示符和真实输出保留。远端关闭回显时不需要额外处理。
- 每连接同时只跟踪一个命令，最长 120 秒；与客户端等待上限分别计时。`ssh.exec` 的作业上限为 30 分钟，最多 8 个并行作业。
- 源命令最多 4096 字节，转义包装后还受 4000 字节 PTY 单行上限约束。
- 结果字段：`exitCode`、`encoding`、`text` 或 `data`、`bytes`、`originalBytes`、`droppedBytes`、`limit`、`truncated`、`syntaxError`、`durationMs`。UTF-8 终端文本返回 `text`，保留 ANSI、换行和常见终端控制字符；NUL、其他二进制控制字节、无效/截断 UTF-8，或 JSON 转义后超过结果预算时返回无损 base64 `data`。捕获仍限 8 KiB；字节统计按原始字节计算。非零退出的 MCP 结果 `isError=true`。
- 取消先发送 Ctrl+C，最多等待 2 秒确认结束标记。确认成功为 `stopped`；否则返回 `unknown` 并自动重建 PTY Shell，连接保持可用。可用 `ssh.reset` 主动重建。
- 断线、结束标记缺失或跟踪到期也可能 unknown；不推测成功、不自动重发。跟踪到期会自动重建共享 PTY，并发送 `ssh.status(reset=true)`；如果重建失败则连接关闭，需要重连。`ssh.exec` 作业不受此限制。

### exec 输出与进度

`ssh.exec` 的结果包含 `exitCode`、`signal`、`timedOut`、`cancelled`，以及最多 4 KiB 的 `stdout`/`stderr` 预览。MCP 文本摘要包含两条流的有界预览，不因存在退出码而漏掉 stdout/stderr；整体摘要按 JSON 转义成本限制为约 2 KiB。完整流保存在有界作业缓冲中（每流 64 KiB、最多 8 个作业），通过 `ssh.output` 用 `stdoutCursor`/`stderrCursor` 读取，每流每次最多 4 KiB，返回 `done`、`exitCode`、`gap`、`more`。取消会关闭通道；通道关闭确认后返回 `stopped`，否则 `unknown`。作业缓冲随连接销毁。

## 输出与限制

每连接回放缓冲 256 KiB，每块最多 4 KiB，`ssh.read` 单次最多 8 KiB；事件为 `ssh.terminal.output`、`ssh.status` 和 `ssh.job`，携带连接 ID 和游标。`ssh.job` 在 exec 作业开始和结束时各发一次，只含 `operationId`、源命令和 `done`，不携带输出；前端据此渲染终端底部的作业行，点击后在该行下方展开作业视图，通过 `ssh.output` 按游标取流。`ssh.status` 事件在重建 Shell 后带 `reset=true`；`ssh.status` 操作本身只报告当前配置与连接 ID。前端先订阅再回放，缺口时恢复；输入队列上限 16 KiB，控制队列 64 项，终端滚动缓存 3000 行。

作业选择区默认只占一行，按作业开始事件的先后从新到旧排列，完成事件不改变顺序。右侧按钮展开完整列表；列表内容高度不超过 `min(180px, 30vh)`，超过时可用滚轮上下查看。选中作业自动收起列表，折叠行只保留从最新作业到选中作业的区间，并滚动至末端，让选中作业位于最右侧；更早的作业仍可重新展开查看。折叠时滚轮可横向浏览该区间。作业状态刷新保持列表节点、键盘焦点和滚动锚点，新作业到达或窗口变窄时保持选中项可见。作业输出视图独立于列表的展开状态，仍保留原有高度上限与内部滚动。

连接错误带稳定阶段：`tcp`、`handshake`、`host_key`、`auth`、`channel`、`pty`、`shell`、`connect`。指纹不匹配使用 `ssh_host_key_mismatch`；远端 Shell 退出使用 `ssh_remote_shell_exited` 并附退出码，网络中断使用 `ssh_connection_lost_before_completion`。

指纹每次连接确认，不持久化。尚无私钥、ssh-agent、MFA、SFTP、端口转发、自动重连和凭据保存；命令块选择与上下文链路也未接通。秘密和背压规则见 [运行时通信](runtime-communication.md)。

## 验证

`cargo test -p coterm-plugin-ssh` 使用本地测试 SSH 服务，覆盖指纹/密码失败、多实例、真实插件进程、审批、秘密隔离、共享 PTY、输出缺口和 MCP。远程服务器及真实桌面验收需单独执行，最近验证范围见 [SSH 开发进度](../progress/ssh-plugin.md)。
