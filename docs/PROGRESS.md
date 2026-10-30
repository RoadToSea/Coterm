# 项目进度

更新：2026-10-28。阶段：**Windows 首个 Alpha 的语言切换与离线插件安装准备**；插件可靠性与跨平台发布继续独立迭代。

## 已有能力

| 范围 | 当前状态 |
|---|---|
| 桌面与插件运行时 | 插件发现、坏包诊断、启停、多实例、可选前端挂载 |
| Alpha 使用入口 | 简体中文/English 切换与偏好保存；宿主和 SSH 前端同步更新；离线 `.coterm`/ZIP 预览、确认、安装与即时发现 |
| 操作与公共交互 | 来源/scope/schema 校验、提案审批、显式 Bypass、宿主公共交互、查询与取消；账本有界、仅保留本次启动 |
| SSH | 密码认证、指纹确认、共享 PTY、命令输出/退出码、回放与事件 |
| 串口 | 人工 Shell 按键直发、UART 调试发送框，保留字节读写、HEX/行结束符、DTR/RTS、收发计数与回放；仅 MCP 提供命令列表队列，顺序、间隔与取消通过 Windows 虚拟串口配对验收；兼容 HHD 驱动不支持 flush；列表的物理板卡发送节奏仍待验收 |
| MCP | 唯一对外入口为 Stdio（桌面 `mcp stdio`）；私有 Token 桥、插件原生 Tools、实例发现与操作路由；HTTP 端点已移除 |
| SDK 与 UI | 后端控制协议 v1.0、统一编解码与握手校验、前端 SDK v1.0、通用块列表、版本化宿主订阅 |
| 本地终端 | 已接入当前运行时，仅有实例/配置与插件信息，尚无 PTY |

## 基座完成度（不含插件）

**已完成 Windows 上运行可信本地插件的首版基座。** 不包含通用流、沙箱、插件质量和跨平台发布承诺。

| 收尾项 | 完成情况 |
|---|---|
| SDK 与公共交互 | SDK v1.0；文本、选择、确认与秘密输入使用宿主对话框，含取消、超时及卸载回收；流能力显式不支持 |
| 状态保存与恢复 | Windows DPAPI 加密配置和引用，原子保存顺序、选中实例与侧栏布局；显式逐项/整体恢复，不重放操作；账本仅保留本次启动 |
| 生命周期 | 崩溃移除运行态、实例标为 failed，清理审批/交互；运行中结果为 unknown，可重建/删除；退出能回收控制请求卡住的子进程 |
| 交付 | 跟踪锁文件，Windows CI/标签发布流程；修正开发包平台声明；真实 Tauri、NSIS 安装/启动/卸载验收 |

非 Windows 暂仅保留会话内工作区，不回退明文存盘。`subscribeStream`、二进制流/StreamRouter、窗口信用、多轮后端交互、完整 MCP、签名和市场属于后续范围。

## 优先待办

### P0：插件可靠性

三项串口 P0 已于 2026-10-30 完成，详见[串口硬件验收](progress/2026-10-30-serial-hardware.md)：

- ~~**串口资源回收**~~：读取线程有停止标志与 join（上限 2 秒），析构也置位；阻塞打开移入阻塞线程池，5 秒 timeout 可中断。12 轮连接/断开压力与销毁后重连通过。
- ~~**串口输入脱敏**~~：`serial.write` 标记为敏感输入，MCP 镜像沿用该标记。
- ~~**Unix 串口支持**~~：接受 `/dev/...` 设备节点，仍拒绝路径穿越；Unix 真机运行仍待补。

### P1：运行时与交付

- 继续补插件模板、独立语言实现验证与 manifest 扩展规则。
- 扩展流接口时，用持续订阅型测试插件验证窗口信用、背压、取消与数据缺口。
- 跨平台发布另补 CI、安装包和系统依赖验证；核验 macOS 图标，当前 `icon.icns` 仅 8 字节。
- 插件验收另覆盖远程 SSH 和物理串口，不与基座验收混算。
- 2026-10-26 已完成一次真实远程 SSH / MCP 运维验证：用户与组管理、Docker 安装、Mihomo 部署及 HTTP/SOCKS5 直连检查；会话名额、审批、结果解析和共享 PTY 的使用问题见 [实测记录](ssh-mihomo-field-test.md)。认证交互、长任务超时和物理串口见 2026-10-30 硬件验收记录。
- 2026-10-26 第二轮真实 SSH 边界测试补充发现：顶层 Shell 语法错误会退出 PTY；取消未确认会长期阻塞连接，需断开重连；错误分类无法区分网络、握手、指纹和 Shell 阶段；命令包装会进入回放；输出在 8192 字节截断。详见 [实测记录](ssh-mihomo-field-test.md)。
- 2026-10-27 使用真实 Tauri 程序回归修复：语法错误返回结构化结果且保持连接、取消后连接可继续执行、结果提供原始/丢弃字节数、操作收据去重、SSH 包装从回放隔离；详见 [实测记录](ssh-mihomo-field-test.md)。
- 2026-10-27 已修复上述 SSH/MCP 边界问题：命令执行前先做静态语法检查，避免登录 Shell 因语法错误退出；取消未确认时自动重建 PTY 并新增 `ssh.reset`；连接错误改为稳定阶段码（`ssh_host_key_mismatch`、`ssh_remote_shell_exited` 等）；包装回显从回放中移除；结果新增 `text/base64` 双编码、原始/丢弃字节数与截断上限。新增 `ssh.exec` 独立通道、`ssh.output` 作业进度、`ssh.system_info`/`ssh.file_info` 只读工具；MCP 工具结果改为 `structuredContent` 加摘要文本，`_meta` 收据不再重复完整输出；MCP 面板可列出并按前缀关闭会话，429 附恢复指引与 `Retry-After`。SSH 与 MCP 新增 6 个回归测试，远端服务器真实验收仍需单独执行。
- 2026-10-29 SSH Agent 易用性：共享 PTY 命令超时后自动重建 Shell；新增有界 UTF-8 `ssh.read_file`/`ssh.write_file`（写入走审批及原子替换、无 sudo）；MCP `ssh.command`/`ssh.exec` 接收有序 `commands` 列表并在首个失败时停止。MCP 首选 Stdio、前端使用 Tauri Host API；私有 Stdio Bridge 已由桌面面板按需启停并交付客户端配置，桌面可执行文件的 `mcp stdio` 子命令即适配器，无需单独分发 CLI。
- 2026-10-30 串口插件真实硬件验收：在 COM6（RISC-V Linux 板卡）验证登录交互、`uname -a` 往返、HEX 渲染、收发计数与 DTR/RTS；修复 TX 计数漏算行结束符；界面验收脚本改为不依赖界面语言。三项 P0 关闭，详见[记录](progress/2026-10-30-serial-hardware.md)。
- 2026-10-29 MCP 只保留 Stdio：桌面 MCP 面板按需启停私有 Token 桥并交付可粘贴的 `mcpServers` 配置，`coterm-desktop mcp stdio` 即适配器，不创建窗口或运行时；Token 只经私有命令返回面板，停用或退出即失效。移除 Streamable HTTP 端点、`axum`/`reqwest`/`futures` 依赖及面板中的地址与会话列表，SSH 的 HTTP 回归改为第二个桥客户端。仓库内的 `coterm-mcp-stdio` 保留为开发回归入口。

### P2：功能与生态

- 本地 PTY、SSH 密钥/MFA/SFTP/转发、串口流控/热插拔、MQTT/CAN/Modbus。
- 命令块选择到 MCP 上下文的闭环；Resources、Prompts、订阅、进度和协议级取消。
- 插件签名、权限与前端隔离、秘密物理通道、覆盖安装/卸载/更新、回滚和市场；离线首次安装已接入，发布流程见 [Alpha 交付说明](alpha-release.md)。

## 最近代码验证

### 2026-10-28 Windows 插件控制台修复

- 用户报告点击创建实例后出现空白黑色窗口；使用旧版 Smoke NSIS 与新增原生断言复现，插件自报 `consoleAttached=true`。其 stdout 已接入宿主协议管道，黑色窗口不提供交互内容。
- 根因是 GUI 宿主启动控制台子系统插件时未禁止控制台分配。Windows 启动路径增加 `CREATE_NO_WINDOW`，保留标准输入/输出协议管道、诊断 stderr 与退出回收；非 Windows 路径不变，无新增依赖或权限。
- 相同真实 Tauri 回归修复后通过，确认 `consoleAttached=false` 且握手、实例操作、离线安装、重启和退出回收正常。另增加真实 SSH 配置界面提交、后端握手及未连接终端渲染验收，不发起网络连接。
- `cargo check --workspace --locked`、Clippy、99 个 workspace 测试通过，1 个硬件测试 ignored；桌面前端、Smoke 与产品 Release NSIS 构建通过，两类安装包均完成临时目录安装与卸载。首次与 Release 构建并行运行测试时，本地插件 2 秒启动用例耗时 2.75 秒失败；构建结束后全 workspace 串行重跑通过，未放宽测试阈值。
- 原生回归的运行前提新增 SSH `dist` 构建，README 与现有 CI 命令保持一致。此次补充的是 GUI 宿主启动边界，不能以控制台测试父进程下的启动结果替代。

### 2026-10-28 Alpha 使用入口

Windows x86_64：新增语言切换与离线安装，尚未创建 Alpha 发布标签。

| 检查 | 结果 |
|---|---|
| Cargo check / clippy --all-targets | `--locked` 检查通过，Clippy 无告警；新增 Rust 文件通过 rustfmt 检查 |
| Cargo workspace tests | 99 通过，1 个硬件用例 ignored；含离线包路径/符号链接/限额、坏包、平台不符、冲突保留、取消清理、重启发现等 8 个新增测试 |
| pnpm test | 35 通过，0 跳过；真实浏览器验证中英文切换保持 SSH 终端对象、语言偏好恢复、安装预览/信任确认、错误与取消 |
| 桌面与 SSH 构建 | `pnpm build`、桌面 Release NSIS 和 `pnpm plugin:ssh:build --release` 通过 |
| 真实 Tauri / WebView2 | 独立 `com.coterm.smoke` Release NSIS 安装后运行：原生文件选择取消、真实 ZIP 校验/预览、确认安装、即时列出与重启发现；已有多实例、审批、交互、加密恢复和退出回收回归通过 |
| Windows NSIS | 本轮重新构建 Smoke 与产品安装包；隔离临时目录中的 Smoke 安装、启动与卸载、产品安装与卸载均通过 |
| 离线分发 | `plugin:pack` 已生成并重新校验 SSH `.coterm`；发布流水线增加独立 SSH 离线包产物 |

本轮未重新执行远程 SSH、物理串口或 macOS/Linux 安装；产品标识下的首次连接仍需发行前人工验收，不将 Smoke 测试插件算作真实 SSH 插件实测。全仓 `cargo fmt --all -- --check` 仍报告既有文件的格式差异，本轮未扩展修改无关代码。

### 2026-10-26 基座收尾记录

2026-10-26，本轮基座首版收尾；Windows x86_64，Cargo 1.97.0、Node.js 22.22.2、pnpm 11.0.8。

| 检查 | 结果 |
|---|---|
| Cargo check / clippy --all-targets | 通过，Clippy 无告警 |
| Cargo workspace tests | 76 通过，1 个硬件用例 ignored；含控制契约、加密保存、崩溃恢复与卡住请求的退出回收 |
| 桌面、SSH、串口前端构建 | 通过 |
| pnpm test | 28 通过，0 跳过，含真实浏览器测试 |
| 真实 Tauri / WebView2 | 多实例、SDK 公共交互、无 UI 秘密交互、慢消费者/缺口、崩溃/重建、加密恢复、Bypass 重置及退出回收通过 |
| Windows NSIS | 隔离标识安装、启动与卸载；产品安装包构建及安装/卸载通过 |

浏览器测试使用 IPC 替身；真实 Tauri 验收使用独立 `com.coterm.smoke` 工作区和无硬件测试插件。CI 流程已加入，云端结果以 Actions 为准。以上基座回归未覆盖远程 SSH、物理串口、macOS/Linux 安装包；另有同日真实远程 SSH 运维验证，范围见 [实测记录](ssh-mihomo-field-test.md)。普通回归不打开或写入物理串口。
