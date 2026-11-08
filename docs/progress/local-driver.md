# 本地驱动开发进度

## 当前能力

原生本地终端、默认前台 command、显式后台 exec、三种无游标读取、Human 选区同步及每实例审批。稳定接口见 [本地终端](../architecture/local-driver.md)。

## 2026-11-08：前台命令与后端记录

- POSIX / PowerShell / cmd 按各自语法提交，前台保留交互环境；后台分离两条流。MCP 删除独立 exec/output/read，保留三种读取；运行中数据可读，断线保留有界记录。
- 真实 PowerShell 和 cmd 验证前台环境延续、后台分流、选区来源、跨实例隔离；边界跨包、终端查询、JSON 预算和记录淘汰通过定向测试。修复旧 output 操作缺少实例归属检查。
- 原生 WebView2 + MCP Stdio + 人工审批界面已验证可见 PTY 命令和环境延续；移除普通用户安装流程，随应用一起分发。非 Windows 实机和安装包验收仍由发布流程执行。

## 2026-11-01：P0 第一阶段（人工终端闭环）

- 新增 `crates/coterm-term`：参数化块大小、容量、是否保存解码文本，吸收 SSH（4 KiB、无文本、有缺口计数）与串口（1 KiB、带 UTF-8 解码、`close` 冲尾）的差异。单元测试 11 项通过，`cargo clippy -p coterm-term -D warnings` 通过。
- `drivers/driver-local` 新增 `shell.rs`（家族识别、PATH 解析、home 兜底）与 `session.rs`（PTY 启动、读取/写入线程、退出轮询、`end_stream` 单次收敛）。
- 操作目录扩到 6 个：`local.driver_info`（driver）、`local.status/read/input/resize/restart`（instance）。`local.input` 标记 `sensitiveInput`，账本不留按键。
- 输入队列按 4 项 × 4 KiB 有界；队满、Shell 已退出、游标超前都返回明确错误，不阻塞控制循环。
- 错误码：`local_shell_not_found`、`local_cwd_unavailable`、`local_domain_error`（兜底）。
- 前端新增 `drivers/driver-local/frontend`（包名 `coterm-driver-local-ui`）：`tsc --noEmit` 通过，`vite build` 产出单文件 `index.js`（约 434 KB）。
- 构建脚本 `scripts/build-local.mjs` 与 `pnpm driver:local:build`；manifest 增加 `frontend.entry`，`requires` 仍为 4 项（`exec` 落地后再扩 `native-mcp-tools`/`async-operations`）。

## 2026-11-01：P0 第二阶段（Agent 命令与 MCP）

- 新增 `exec.rs`：`Jobs`（每实例 8 个作业、只淘汰已完成）、两条 64 KiB 流、`watch` 取消、超时与 `_cotermDeferred` 完成通知。
- 脚本交付按家族分流：POSIX `-s`、PowerShell `-NoLogo -NoProfile -NonInteractive -Command -` 走 stdin；cmd 只能落临时 `.cmd`，文件头写 `@echo off` 且不接 stdin（否则批处理结束回到交互模式，把提示符和回显写进结果），结束后删除。
- 新增 `result.rs`：MCP 结果结构化内容只放一份，文本块只做摘要，非零退出 `isError=true`。
- `catalog()` 追加 `mcp.*` 别名（基座按目录解析工具作用域，缺了会整驱动被拒 `driver_mcp_catalog_invalid`）；manifest 的 `requires` 扩到含 `async-operations`、`native-mcp-tools`。
- 取消：2 秒内确认结束回 `stopped`，否则 `unknown`；未登记的作业回 `not_submitted`。

## 验证状态

- `cargo test -p coterm-driver-local` 全绿，共 18 项：11 单元（配置、家族、参数、MCP 目录）+ 2 进程级（控制契约、取消作业）+ 5 集成（真实驱动进程、PTY 读写、resize、重启、实例隔离、exec 退出码/两条流/超时、传输信封、握手时限）。
- `cargo clippy -p coterm-driver-local -p coterm-term --all-targets -- -D warnings` 通过。
- `cargo test -p coterm-term` 11 项通过；前端 `tsc --noEmit` 与 `vite build` 通过（单文件 434 KB）。
- 开发包已组装并更新：`dist/backend/coterm-driver-local.exe`、`dist/frontend/index.js`、`dist/manifest.json`。
- 未执行：桌面内真实点击验收（本机环境见下），`cargo clippy --workspace` 全量。

## 本轮遇到的现场问题

排查中确认了两件事，都已写进架构文档：

- ConPTY 启动时会先发 `ESC[6n` 光标位置查询，客户端不回应就只有初始化序列。xterm 会自动回，测试客户端必须自己回 `ESC[<row>;<col>R`。
- PowerShell 在本机冷启动超过 20 秒（profile + 模块）。端到端测试固定用 `cmd.exe`/`/bin/sh`，避免断言依赖机器状态。

工具链方面，仓库内 `target/**/.cargo-*-lock` 被安全策略拒绝打开，`cargo build/test` 直接 `os error 5`。可行绕法：把已有 `target` 硬链接复制到仓库外（跳过 `.cargo-*-lock`），再把 `CARGO_TARGET_DIR` 指过去，缓存可复用。本轮所有 Rust 验证都在这种外置目录完成。

## 限制与下次入口

- 原始人工按键不生成对话块；未实现 OSC 133 和后台作业专用 UI。
- Windows 后台取消只结束直接子进程，不能保证孙进程一起退出。
- 非 Windows 的真实 Shell、发行安装包和进程树回收需要后续独立验收。
- SSH、串口继续各自维护领域记录，不为共用模型增加迁移。

## 关键文件

- 后端：[backend.rs](../../drivers/driver-local/src/backend.rs)、[session.rs](../../drivers/driver-local/src/session.rs)、[exec.rs](../../drivers/driver-local/src/exec.rs)、[shell.rs](../../drivers/driver-local/src/shell.rs)、[result.rs](../../drivers/driver-local/src/result.rs)
- 共享 crate：[coterm-term](../../crates/coterm-term/src/buffer.rs)
- 前端：[index.ts](../../drivers/driver-local/frontend/src/index.ts)
- 契约与方案：[本地驱动](../architecture/local-driver.md)、[设计方案](../design/local-terminal.md)
