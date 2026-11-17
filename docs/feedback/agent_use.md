# MCP PTY 使用边界

SSH／Local 的 `coterm_terminal_command` 省略 `mode` 时使用隔离 `exec`，不向桌面可见的共享 PTY 写入命令。复现验证：Local `cmd` 在 `set /p` 提示符等待输入时，省略 `mode` 的 MCP 命令仍独立完成并返回输出。

显式指定 `mode: "terminal"` 会复用可见 PTY，以保留当前目录、环境和交互状态。当前实现不能可靠判断用户是否在 PTY 中手动启动了交互程序；若该程序正在前台，它可能接收 Agent 输入。只有确认 PTY 正处于 Shell 提示符且确需沿用其状态时才指定 terminal。跨平台忙碌前置检查仍未实现。

审批后的结果通过原 `operationId` 查询；完成记录包含驱动结构化输出，不需要为同一结果再调用 `coterm_read_output`。`coterm_read_output` 用于读取终端最近行、命令对话或人工选区。

SSH sudo 秘密由桌面交互提供，秘密交互有 60 秒期限；文件操作的 `timeoutMs` 从作业执行阶段计时，不覆盖此前的审批或秘密等待。等待超时后查询原 operation，不重复提交。
