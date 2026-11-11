# 本地驱动开发进度

## 当前能力

原生本地终端、默认前台 command、显式后台 exec、三种无游标读取、Human 选区同步及每实例审批。稳定接口见 [本地终端](../architecture/local-driver.md)。

## 2026-11-11：进程内运行

- 后端作为库链接到桌面，控制测试直接使用 DriverTask，端到端测试通过真实进程内运行时操作 PTY 与后台作业。移除独立驱动程序与临时测试包。
- Jobs 析构主动发送取消，避免任务中持有的 Arc 让后台子进程在驱动退出后继续运行。前台状态、后台取消、隔离读取和真实 PowerShell/MCP/审批链路回归通过。
- 桌面 EXE 在空目录独立运行验收通过；跨平台原生进程树回收仍需验收。整体迁移与验证见[项目进度](project.md)。

## 2026-11-08：前台命令与后端记录

- POSIX / PowerShell / cmd 按各自语法提交，前台保留交互环境；后台分离两条流。MCP 删除独立 exec/output/read，保留三种读取；运行中数据可读，断线保留有界记录。
- 真实 PowerShell 和 cmd 验证前台环境延续、后台分流、选区来源、跨实例隔离；边界跨包、终端查询、JSON 预算和记录淘汰通过定向测试。修复旧 output 操作缺少实例归属检查。
- 原生 WebView2 + MCP Stdio + 人工审批界面已验证可见 PTY 命令和环境延续；移除普通用户安装流程，随应用一起分发。非 Windows 实机和安装包验收仍由发布流程执行。

## 2026-11-01：P0 第一阶段（人工终端闭环）

- 新增 `crates/coterm-term`：参数化块大小、容量、是否保存解码文本，吸收 SSH（4 KiB、无文本、有缺口计数）与串口（1 KiB、带 UTF-8 解码、`close` 冲尾）的差异。单元测试 11 项通过，`cargo clippy -p coterm-core -D warnings` 通过。
- `drivers/driver-local` 新增 `shell.rs`（家族识别、PATH 解析、home 兜底）与 `session.rs`（PTY 启动、读取/写入线程、退出轮询、`end_stream` 单次收敛）。
- 操作目录扩到 6 个：`local.driver_info`（driver）、`local.status/read/input/resize/restart`（instance）。`local.input` 标记 `sensitiveInput`，账本不留按键。
- 输入队列按 4 项 × 4 KiB 有界；队满、Shell 已退出、游标超前都返回明确错误，不阻塞控制循环。
- 错误码：`local_shell_not_found`、`local_cwd_unavailable`、`local_domain_error`（兜底）。
- 前端新增 `crates/coterm-driver/frontend/local`（包名 `coterm-driver-local-ui`）：`tsc --noEmit` 通过，`vite build` 产出单文件 `index.js`（约 434 KB）。
- 构建脚本 `scripts/build-local.mjs` 与 `pnpm driver:local:build`；manifest 增加 `frontend.entry`，`requires` 仍为 4 项（`exec` 落地后再扩 `native-mcp-tools`/`async-operations`）。

## 2026-11-01：P0 第二阶段（Agent 命令与 MCP）

- 新增 `exec.rs`：`Jobs`（每实例 8 个作业、只淘汰已完成）、两条 64 KiB 流、`watch` 取消、超时与 `_cotermDeferred` 完成通知。
- 脚本交付按家族分流：POSIX `-s`、PowerShell `-NoLogo -NoProfile -NonInteractive -Command -` 走 stdin；cmd 只能落临时 `.cmd`，文件头写 `@echo off` 且不接 stdin（否则批处理结束回到交互模式，把提示符和回显写进结果），结束后删除。
- 新增 `result.rs`：MCP 结果结构化内容只放一份，文本块只做摘要，非零退出 `isError=true`。
- `catalog()` 追加 `mcp.*` 别名（基座按目录解析工具作用域，缺了会整驱动被拒 `driver_mcp_catalog_invalid`）；manifest 的 `requires` 扩到含 `async-operations`、`native-mcp-tools`。
- 取消：2 秒内确认结束回 `stopped`，否则 `unknown`；未登记的作业回 `not_submitted`。

## 验证入口

当前全量验证结果见 [项目进度](project.md)。定向回归：`cargo test -p coterm-driver`、`cargo test -p coterm-core`；前端入口为 [Local 浏览器回归](../../tests/local-terminal-browser.test.mjs)，原生链路运行 `pnpm test:native`。

ConPTY 初始化可能请求光标位置，测试客户端需响应终端查询；真实 PowerShell 冷启动耗时与 profile 相关，测试应等待可交互状态。

## 限制与下次入口

- 原始人工按键不生成对话块；未实现 OSC 133 和后台作业专用 UI。
- Windows 后台取消只结束直接子进程，不能保证孙进程一起退出。
- 非 Windows 的真实 Shell、发行安装包和进程树回收需要后续独立验收。
- SSH、串口继续各自维护领域记录，不为共用模型增加迁移。

## 关键文件

- 后端：[backend.rs](../../crates/coterm-driver/src/local/backend.rs)、[session.rs](../../crates/coterm-driver/src/local/session.rs)、[exec.rs](../../crates/coterm-driver/src/local/exec.rs)、[shell.rs](../../crates/coterm-driver/src/local/shell.rs)、[result.rs](../../crates/coterm-driver/src/local/result.rs)
- 共享 crate：[coterm-term](../../crates/coterm-core/src/terminal/buffer.rs)
- 前端：[index.ts](../../crates/coterm-driver/frontend/local/src/index.ts)
- 稳定契约：[本地驱动](../architecture/local-driver.md)
