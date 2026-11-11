# 本地终端驱动

本地 PTY、Shell 探测、命令边界、后端有界记录和界面归本地驱动；基座负责进程隔离、实例路由、审批和 MCP。

创建实例只登记 `shell/cwd/family`，不会启动进程。人工显式启动或重启 PTY；读取不执行命令。Shell 退出后保留实例和后端记录，重启生成新的 shellId，旧事件与旧选区不能写入当前连接。

## 命令和读取

- `local.command({commands:string[],mode?:"terminal"|"exec"})`：1–64 项，源文本合计 64 KiB；默认在当前可见 PTY 执行，保留目录和环境。显式 exec 才创建独立后台 Shell，不继承交互状态。运行时等待超时不改通道、不重放。
- POSIX 用同一 Shell 的 eval，PowerShell 用当前作用域 ScriptBlock；cmd 经受限临时批处理文件 call 回当前终端，结束清理文件并恢复 echo on。列表首个失败停止；前台原始 stderr 已合并，返回空 stderr。Shell 主动退出、断线或取消未观察到结束时保留未知状态。
- `read_lines(lines=100)`：1–1000，后端终端最近逻辑行；保留 ANSI，并非 xterm 屏幕行。
- `read_dialogue_pairs(count=2)`：1–32，最新在前，覆盖 API 前台和后台命令；始终有 stdin/stdout/stderr、mode、streamMode、status、exitCode、truncated 和 error，未知为 null。
- `read_selected_lines()`：读取仅 Human 前端能同步的选中文本；按 shellId 校验，最多 16 KiB，清除、退出和卸载清理。

MCP 只公开 status、command、上述三种读取及驱动信息。原生 read/output/exec 为内部兼容与回放入口，不注册成 MCP 游标工具。API 命令经过每实例审批，原始人工按键不作为命令记录，不据此猜测密码或命令边界。

## 有界与取消

每实例最多 32 条记录，每条流保留 16 KiB，单次 JSON 快照 12 KiB，淘汰和截断显式返回。PTY 回放 256 KiB；后台最多 8 个作业，每流原始缓冲 64 KiB。前台边界令牌与命令回显分开，跨包识别；回显过滤仍转发终端查询，避免阻塞 PowerShell 光标报告交互。输出伪造边界不构成认证或执行成功证明。

前台取消发送 Ctrl-C，但不以此证明终止；未确认时返回 unknown，执行通道保持占用直至边界或用户重启。后台复用子进程取消与超时。历史只驻内存，不新增落盘日志；配置使用工作区安全保存项。

实现：[领域路由](../../crates/coterm-driver/src/local/backend.rs)、[命令与记录](../../crates/coterm-driver/src/local/command.rs)、[PTY](../../crates/coterm-driver/src/local/session.rs)、[后台作业](../../crates/coterm-driver/src/local/exec.rs)。
