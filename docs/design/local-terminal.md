# 本地终端驱动方案

日期：2026-11-01。

状态：P0 第一、二步（人工终端 + `local.exec`/MCP）已实现并完成本地回归；共享 crate 迁移与共享 PTY 命令待做。实施与验证范围见 [本地驱动进度](../progress/local-driver.md)，稳定约束已同步到 [本地驱动](../architecture/local-driver.md)。

## 目标

`drivers/driver-local` 提供本地 PTY Shell 实例：人工终端与 Agent 共享同一个实例，能力与 SSH 终端对齐，但连接目标是本机而非远端。

- 人工：真实 PTY、窗口尺寸、快捷键、搜索、原生 Shell 语义与用户 profile。
- Agent：在实例所属 Shell 中执行脚本，拿到退出码与有界输出；写操作默认走基座审批。
- 多实例并存，实例之间不共享进程、目录或环境。

不做：文件读写工具、系统信息工具、提权、端口转发、凭据保存。这些本地场景由 Shell 命令自身完成（`cat`、`Get-Content`、`dir`）。理由：本地驱动若再暴露一套文件接口，等于给 Agent 增加一条绕过审批语义的读写通径，而终端本身已经具备该能力且行为对用户透明。

## 现状与缺口

现有实现约 300 行：只有 `create_instance`/`destroy_instance` 与 `local.driver_info`，握手声明 4 项特性，没有 PTY，没有前端入口。

关键阻塞：`manifest.json` 缺少 `frontend` 时，基座按 `hasFrontend === false` 渲染"该驱动不提供配置界面"，不会调用 `create_instance`。因此**当前用户无法创建本地实例**，配置前端是 P0 的必需项而不是可选项。

## 决策

1. **双路径，exec 优先**。`local.exec` 用独立子进程 + 管道，退出码与两条流精确、跨 Shell 家族行为一致，作为 Agent 主路径。`local.command`（在共享 PTY 上用标记跟踪命令）二期实现且仅支持 POSIX 家族：PowerShell 的 PSReadLine 语法高亮与整行重绘会破坏 SSH 现用的回显剥离判据，需要单独设计边界协议，不在 P0 冒险。
2. **不新增文件类 MCP 工具**，理由见「目标」。
3. **抽取共享 crate `crates/coterm-term`**，并把 SSH、串口一起迁移，避免出现第三份输出缓冲实现。迁移必须字节语义等价，不改变既有返回值与事件结构。
4. **P0 全栈闭环**：桌面里能真正打开本地终端并使用 Agent 工具。

## 共享 crate：crates/coterm-term

SSH 与串口的缓冲语义并不相同，抽象需要同时容纳两者：

| 维度 | SSH（session.rs） | 串口（session.rs） |
|---|---|---|
| 块大小 | 4 KiB | 1 KiB |
| 容量 | 64 KiB（作业流）/ 256 KiB（终端回放） | 固定上限 |
| 是否保存解码文本 | 否，返回原始字节 | 是，`push` 返回 UTF-8 文本 |
| 跨块 UTF-8 边界 | 不处理 | `Utf8Decoder` 处理，`close` 冲刷尾字节 |
| 缺口计数 | 有 `total`/`dropped` | 无 |

接口草案：

```rust
pub struct OutputBuffer { /* chunks, bytes, sequence, limit, chunk_bytes, total, dropped, closed, decoder */ }

impl OutputBuffer {
    pub fn new(limit: usize, chunk_bytes: usize, decode: bool) -> Self;
    pub fn push(&mut self, data: &[u8]) -> Option<String>;   // decode=false 时返回 None
    pub fn close(&mut self) -> Option<String>;
    pub fn cursor(&self) -> u64;
    pub fn collect(&self, after: u64, limit: usize) -> Result<Page>;
}

pub struct Page {
    pub cursor: u64,
    pub data: Vec<u8>,
    pub text: Option<String>,
    pub gap: bool,
    pub more: bool,
    pub closed: bool,
    pub dropped: usize,
}
```

同 crate 一并收纳：`Utf8Decoder`、`encode`（UTF-8 文本/无损 base64 选择与字节统计）、十进制游标解析。

作业表（上限淘汰、取消通道）暂留在 `driver-local/src/exec.rs`：它和 `RuntimeError`、`watch` 取消、一次性完成通知耦合，等迁移 SSH 的 exec 作业时再确认能否上移到 crate。

迁移顺序（每步独立提交、独立回归，不合并）：

1. 建 crate 并加单元测试，用例直接从两侧现有边界整理：游标超前、容量淘汰产生 gap、块内不足、UTF-8 跨块与 `close` 尾字节、空块。
2. `driver-local` 先使用，验证抽象可用。
3. 迁移 SSH：`cargo test -p coterm-driver-ssh` 全绿，事件 payload 与 `ssh.read`/`ssh.output` 返回字段不变。
4. 迁移串口：`cargo test -p coterm-driver-serial` 全绿。串口原先不统计缺口，迁移后新增的 `dropped` 只能作为可选增量字段，不改变现有返回结构。
5. `cargo check/clippy --workspace` 收尾。

## 后端设计

### 配置与 Shell 探测

```jsonc
{ "shell": "", "cwd": "", "family": null }
```

- `shell` 为空时由后端探测，结果写入实例标签与 `local.status`；非空时按给定路径/名称启动，不做 PATH 之外的拼接。
- 探测顺序：Windows 依次尝试 `pwsh.exe`、`powershell.exe`（PATH 或 `%SystemRoot%` 下的系统路径）、`%COMSPEC%`；Unix 依次尝试 `$SHELL`、`/bin/bash`、`/bin/sh`。全部失败返回 `local_shell_not_found`。
- `family` 默认由可执行文件名推断：`sh/bash/zsh/dash/ksh` → POSIX，`pwsh/powershell` → PowerShell，`cmd` → Cmd，其余 `Unknown`（只允许人工终端，`exec` 返回 `local_shell_family_unsupported`）。显式配置可覆盖探测，用于 `fish`、`nu` 等。
- `cwd` 为空时使用用户 home，启动前校验存在且为目录。
- 校验沿用现有实现：拒绝空 shell、控制字符、超长路径，`deny_unknown_fields` 暴露拼写错误。

### 实例状态

```rust
struct Instance {
    config: Config,
    shell: Option<ShellSession>,   // None 表示已退出或尚未启动
    jobs: JobRegistry,             // 上限 8
}
```

- `create_instance` 只登记配置，不启动进程；首次 `local.read`/`local.input`/`local.exec` 时惰性启动，避免创建实例即占用 PTY。启动失败要让首个操作返回结构化错误，而不是留下半死实例。
- Shell 退出：关闭回放缓冲，发 `local.status { running: false, exitCode }`；实例保留，可 `local.restart` 重建。
- `destroy_instance`/`shutdown`：结束子进程与全部作业，清空缓冲、订阅与待审批提案。

### 操作目录

| 操作 | scope | effect | 说明 |
|---|---|---|---|
| `local.driver_info` | driver | read | 已有，保留 |
| `local.status` | instance | read | Shell、家族、cwd、pid、运行状态、尺寸、游标 |
| `local.read` | instance | read | 按游标回放 base64 字节，报告 gap/more/closed |
| `local.input` | instance | commit | 人工原始按键，`sensitiveInput: true` |
| `local.resize` | instance | draft | rows/cols，10–1000 范围 |
| `local.restart` | instance | commit | 重建 PTY；未完成作业按规则收尾 |
| `local.exec` | instance | commit | 独立子进程执行一段脚本 |
| `local.output` | instance | read | 按游标读取 exec 作业的 stdout/stderr |
| `local.command` | instance | commit | 二期，POSIX 家族，共享 PTY + 标记 |

`local.exec` 输入：

```jsonc
{ "script": "…", "timeoutMs": 120000 }
```

- `script`：1 字节至 64 KiB UTF-8。空脚本、纯空白拒绝。
- `timeoutMs`：1 000–1 800 000，默认 120 000。等待上限只结束等待，不隐式取消；取消必须显式请求，`cancel` 返回 `not_submitted`/`stopped`/`unknown`。
- 不接受 `shell`、`cwd`、`env` 参数。执行体固定为实例配置的 Shell，否则 `local.exec` 会退化成任意进程启动代理，绕过"驱动不提供任意 shell 代理"的基线。

### 脚本交付方式

脚本一律不拼进命令行参数，避免长度上限与多重转义：

| 家族 | 入口 | 交付 |
|---|---|---|
| POSIX | `<shell> -s` | 写 stdin，关闭 stdin |
| PowerShell | `-NoLogo -NoProfile -NonInteractive -Command -` | 写 stdin，关闭 stdin |
| Cmd | `%COMSPEC% /D /S /C <临时 .cmd>` | 写独占临时文件，执行后无条件删除 |

Cmd 没有从 stdin 读脚本的标准方式，只能落临时文件；临时文件用随机名、实例进程权限创建，`finally` 语义保证删除。

子进程在实例 `cwd` 下启动，继承用户环境，注入 `COTERM_INSTANCE` 便于 Shell 侧识别。

### 超时、取消与进程回收

- Unix：`setsid` 建立进程组，取消时先 `killpg(SIGTERM)`，宽限 1 秒后 `SIGKILL`，确认回收再返回 `stopped`。
- Windows：`Child::kill()` 只结束直接子进程，孙进程可能存活。P0 记录为已知限制并在结果中标注 `descendantsMayLive: true`，不做 Job Object 封装；需要时由用户手动结束。
- 未确认回收、超时到期、实例销毁都返回 `unknown`，不自动重放。

### 输出与事件

- 终端回放 256 KiB、块 4 KiB、`local.read` 单次 8 KiB。
- 作业流每流 64 KiB、块 4 KiB、`local.output` 每流每页 4 KiB。
- 输入队列 16 KiB，控制队列 64 项；慢消费者收到 `runtime.gap` 后按游标回放。
- 事件：`local.terminal.output {shellId, data, cursor}`、`local.status {shellId, running, exitCode?, reset?}`、`local.job {operationId, script, done}`。`data` 为 base64、单块不超过 4 KiB。

### 错误码

`local_shell_not_found`、`local_shell_family_unsupported`、`local_pty_unsupported`、`local_shell_exited`、`local_invalid_script`、`local_timeout_out_of_range`、`local_job_limit`、`local_output_cursor_ahead`。沿用现有 `local_domain_error` 作为兜底，新增码逐条映射到 `ErrorCategory`。

## MCP 工具

| 工具 | 映射操作 | effect |
|---|---|---|
| `local.status` | `mcp.local.status` | read |
| `local.exec` | `mcp.local.exec` | commit |
| `local.output` | `mcp.local.output` | read |
| `local.read` | `mcp.local.read` | read |
| `local.driver_info` | `mcp.local.driver_info` | read |

不注册 `local.input`（原始按键含秘密）、`local.resize`、`local.restart`。参数带 `driverInstanceId`，与 SSH 一致。`local.exec` 非零退出时工具结果 `isError: true`，摘要同时包含两条流的有界预览与字节统计。

安全提醒写入工具描述：脚本内容会进入本次会话的有界操作账本（退出即清除），不要在其中拼接密码或 Token；需要交互式认证时由人工在终端完成。

## 前端

新增 `drivers/driver-local/frontend`，包名 `coterm-driver-local-ui`，产出单文件 ES bundle（`vite build --lib`，≤2 MiB）。

`configure(container, api)`：
- Shell 输入框（占位提示"留空自动探测"）+ 常用 Shell 快捷填充按钮 + `cwd` 输入框。
- Shadow DOM 内联样式，只用 `var(--coterm-*, fallback)`，控件圆角，提交期间禁用并抑制重复提交（沿用 SSH/串口的既有约定）。
- 不做原始 JSON 编辑器，不暴露环境变量或启动参数。

`mount(container, api)`：
- xterm + FitAddon + SearchAddon，复用 `@coterm/driver-sdk/components` 的 `terminal-shortcuts`、`terminal-search`、`terminal-theme`。
- 顺序固定：先 `subscribeEvents` 再 `local.read` 回放；输出事件带游标，跳号即重新回放。
- `ResizeObserver` 100 ms 防抖后 `local.resize`；切换标签只隐藏面板，重新显示时重算尺寸。
- 状态条显示 Shell 名、家族、cwd、运行/退出状态与重建按钮。
- 卸载：退订、断开 observer、释放 addon 与 terminal、清空容器。

`i18n`：沿用"英文原文为 key + 中文词典"结构，`api.i18n` 不存在时回退英文，卸载时注销监听。

## 构建与清单

- `drivers/driver-local/manifest.json`：新增 `frontend.entry`，`runtime.requires` 扩为 `instances`、`backend-calls`、`operation-catalog`、`native-mcp-tools`、`async-operations`、`driver-events`；`instances.type` 保持 `local-shell`（现有测试已断言）。
- `scripts/build-local.mjs` 参照串口脚本：`cargo build -p coterm-driver-local` → 前端 `build` → 复制二进制到 `dist/backend` → 按平台写 manifest。
- 根 `package.json` 增加 `driver:local:build`。

## 里程碑

每步独立提交，分支 `feature/driver-local`。

1. `crates/coterm-term` 与单元测试。
2. PTY 会话与人工终端操作（`status`/`read`/`input`/`resize`/`restart`）+ 事件；真实进程测试通过。
3. `local.exec`/`local.output` 与 MCP 工具注册。
4. 配置前端、终端前端、构建脚本与 manifest 扩展。
5. 迁移 SSH、迁移串口、工作区回归。

## 验收

- `cargo test -p coterm-driver-local`：配置校验、家族探测、脚本参数边界、PTY 读写回放、resize、退出与重启、作业上限与游标缺口。
- 真实驱动进程：握手特性、操作目录、实例隔离、`local.exec` 退出码与流、未注册操作被拒。
- 真实 MCP `tools/list` 与工具调用，确认无非零退出的成功结果。
- SSH 与串口迁移后原有测试全绿，事件与返回字段不变。
- `cargo check --workspace`、`cargo clippy --workspace -- -D warnings`、`pnpm build`。
- 桌面实测：创建本地实例、输入输出、多实例、切换标签后尺寸正确、关闭标签不销毁实例、Shell 退出与重启。

## 风险与已知限制

| 风险 | 应对 |
|---|---|
| ConPTY 需要 Windows 10 1809+ | 启动失败返回 `local_pty_unsupported` 并提示升级，不做静默降级 |
| PowerShell 人工终端受用户 profile 的 PSReadLine 影响，显示差异 | 人工终端保留 profile（符合预期），`exec` 固定 `-NoProfile` |
| Windows 取消可能遗留孙进程 | 结果标注 `descendantsMayLive`，文档明示限制 |
| 共享 PTY 命令跟踪在 PowerShell 上不可靠 | P0 不实现，P1 单独设计边界协议 |
| 迁移 SSH/串口引入回归 | 分步提交、逐步全量回归，抽象以两侧现有行为为准而非重写 |
| `local.exec` 是任意代码执行 | 固定实例 Shell、commit + 审批、脚本不拼命令行、账本脱敏提示 |
| 大输出阻塞 | 全链路有界缓冲 + 游标分页 + gap 语义 |

后续待评估：`local.command`（POSIX 共享 PTY）、`shellArgs`（`wsl.exe` 等入口）、OSC 133 命令块、作业 UI。
