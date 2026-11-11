# 网络驱动进度

## 当前能力

- Telnet 与 Raw TCP 作为进程内驱动随桌面分发，支持独立多实例、显式连接、超时、可选自动重连及无认证 SOCKS5／HTTP CONNECT 代理。
- Telnet 支持增量 IAC、NVT、BINARY／ECHO／SGA／NAWS／TTYPE 协商与可选用户名预填。密码由人工终端输入，不保存到连接配置。
- Raw TCP 共享串口的终端／字节工作台，支持文本与任意字节 HEX 收发；终端／字节视图对应 Shell／stream 解释模式。
- 两个驱动使用现有实例审批、配置保存与 MCP 路由。Shell 提供三类无游标读取，stream 只提供 read_lines；后端有界保存接收数据和对话，前端保留显示副本。
- 重连创建新 connectionId，拒绝旧输入及旧审批提案，不重放命令。前端每 2 秒补取状态与遗漏回放，卸载清理订阅和定时器。

## 2026-11-11：驱动接入与公共工作台抽取

- 后端新增 `tcp`／`telnet` 入口，`network` 共享 TCP 生命周期，`stream` 从串口提取接收缓冲、输入编码和提示符观察；无新增 Rust 依赖或宿主接口变更。
- 前端提取公共 xterm 生命周期与终端／字节工作台。SSH 保留命令块和后台作业，Serial 保留实际 UART 设置；Telnet 隐藏字节面板，网络表单不包含串口控制线或 SSH exec 参数。
- 网络命令提案向待审批内存提供完整 operation／args 草稿，批准时重新校验实例、配置版本与连接 ID。普通账本仍由宿主脱敏；Shell 超时和取消保留有界部分结果，并以未知结果收敛，不自动补发。
- 驱动测试覆盖任意字节和 UTF-8 拆包、多实例、审批前不发送、完整草稿、旧提案失效、批次超时／取消、重连不重放、慢消费者与缓存缺口、Telnet 协商、代理握手分片与目标首包。
- Windows 原生 WebView2 → MCP Stdio → 人工审批界面 → 两种网络驱动 → 本机回环 Shell 验收通过；验证后端对话读取、保存配置恢复及恢复后不自动连接。原有 Local PowerShell 原生验收保留并通过。
- 浏览器覆盖显式连接、配置校验与保存失败重试、连接 ID、Telnet 回显／Enter／退格、Raw TCP HEX、事件缺失时状态补取、窄屏及卸载清理。
- 最终验证：工作区全目标 check、274 项 Rust 测试（3 项真实串口硬件用例按原设置忽略）、全目标严格 Clippy 通过；五驱动前端、桌面及普通 debug EXE 构建通过。串行前端全量 103 项全部通过、无跳过；配置、HEX 和 360px 窄窗口截图已复核。
- 收尾：测试进程已退出，已调用 `pnpm debug:clean` 并确认仓库调试目录为空。早期失败用例留下的三个系统临时浏览器 profile（`coterm-layout-mxu1DB`、`coterm-layout-n87wQj`、`coterm-layout-WRX0OE`）递归删除被执行策略拒绝，保留原处，未扩大清理范围。

## 限制与续作入口

- Telnet 与 Raw TCP 均无 TLS；代理仅支持无认证模式，未实现代理账户或通用登录脚本。
- Telnet 仅响应服务端发起的受支持选项协商，不宣称完整 RFC 1143 状态机或所有设备兼容。
- Shell 以完整字面提示符观察边界，忽略末尾空白；提示符不证明执行成功，exitCode 为 null，stderr 无独立通道时为空。人工登录后必须先收到 Shell 提示符。
- stream／HEX 命令仅确认发送，未捕获设备完成结果；接收数据用 read_lines 查询。接收数据、计数、历史与连接状态不跨启动保存。
- 本轮验收使用 Windows 回环服务端和协议夹具，未使用真实远程 Telnet 设备、实际代理部署或 Linux／macOS 实机；发行安装包另行验收。
- 配置、容量和操作边界见[网络驱动架构](../architecture/network-drivers.md)。继续入口：[网络后端](../../crates/coterm-driver/src/network)、[Telnet 编解码](../../crates/coterm-driver/src/telnet/codec.rs)、[网络表单](../../apps/shared/components/network-connection.ts)、[共享工作台](../../apps/shared/components/stream-workbench)、[浏览器验收](../../tests/network-drivers-browser.test.mjs)、[原生验收](../../scripts/test-native.mjs)。
