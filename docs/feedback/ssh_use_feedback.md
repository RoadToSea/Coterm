# SSH MCP 未解决反馈

核对日期：2026-10-30。反馈来自通过 Coterm 桌面与 SSH 插件进行配置文件写入、服务部署和远端排查的使用过程。

## 1. 命令接口仍不接受多行脚本

**使用现象**：通过共享 PTY 执行 `sudo tee ... << 'EOF'` 等多行命令被拒绝。写入受保护配置文件时，Agent 曾需先编码文件，再拼接解码与写入命令。

**剩余限制**：MCP 的 `ssh.command` 与 `ssh.exec` 虽支持 `commands` 列表，每项仍必须是非空单行，最多 4096 字节；共享 PTY 还受转义包装后的 4000 字节单行上限约束。已有 `ssh.read_file` / `ssh.write_file` 可处理 UTF-8 文件内容，但不覆盖直接执行多行脚本，以及通过 sudo 写受保护文件的场景。

**待完善**：明确多行脚本的执行入口和容量限制，避免要求 Agent 手工编码或复杂拼接。涉及提权的文件操作需同时明确审批与秘密输入流程。

约定见 [SSH Agent 工具边界](../architecture/ssh-plugin.md#操作)，校验见 [SSH Backend](../../plugins/plugin-ssh/src/backend.rs) 的 `validate_commands`。

## 2. 独立 exec / 文件通道与共享 PTY 的权限上下文缺少衔接

**使用现象**：用户在桌面终端中通过 `sudo` / `su` 获得的权限，可以被共享 PTY 命令沿用；切到独立 `ssh.exec` 通道后，Agent 使用的是 SSH 登录用户的权限，无法沿用该提权状态。

**剩余限制**：独立通道不继承共享 PTY 的 `cd`、`export` 或提权状态；`ssh.read_file` / `ssh.write_file` 也使用独立通道，尚不支持 sudo。现有操作审批与 SSH 登录秘密交互，没有覆盖反馈期望的“执行中需要提权 → 桌面提示 → 用户授权或输入密码 → 继续操作”流程。

**待完善**：清晰呈现操作使用的权限与环境，并设计显式、受控的提权交互。共享 PTY 与独立通道的差异需要对 Agent 可见，避免工具切换后误判权限，也避免静默重放写操作。

约定见 [SSH 插件](../architecture/ssh-plugin.md)；文件通道实现见 [SSH Session](../../plugins/plugin-ssh/src/session.rs) 的 `read_file` 和 `write_file`。

## 3. ANSI / 控制字符仍使可读输出整体转为 Base64

**使用现象**：带颜色或轻微终端控制序列的文本输出失去 `text` 字段，Agent 必须额外解码 `data`，增加读取步骤和工具调用。

**剩余限制**：当前 `encode_output` 仅将不含特殊控制字符的 UTF-8 输出作为文本；包含 ESC 等控制字符时，整个捕获仍返回 Base64。共享 PTY 的单次结果捕获上限仍为 8 KiB，大输出需要结合截断字段与回放读取处理。

**待完善**：考虑在精确保留原始字节的基础上提供便于 Agent 阅读的文本表示，明确 ANSI、无效 UTF-8 和截断的处理语义；保持分页或增量读取的边界清楚。

约定见 [SSH 输出与限制](../architecture/ssh-plugin.md#输出与限制)；编码与捕获实现见 [SSH Command](../../plugins/plugin-ssh/src/command.rs) 的 `encode_output`、`MAX_CAPTURE`。
