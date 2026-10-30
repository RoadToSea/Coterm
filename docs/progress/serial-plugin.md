# 串口文本与 HEX 接口拆分

本文件跟踪串口插件的接口、人工操作和设备验收。设计约定见 [串口插件架构](../architecture/serial-plugin.md)。

## 契约

- 人工 Shell/UART 文本：`serial.write({text,lineEnding?})`；默认不附加行结束符。
- Agent 文本列表：`serial.command({commands,lineEnding?,intervalMs?})`；与人工文本共用编码函数，保留所有控制字符及多行输入。
- 人工 HEX：`serial.write_hex({hex})`；Agent HEX 列表：`serial.command_hex({commands,intervalMs?})`，项为 HEX 字符串，绝不隐式追加行结束符。
- 文本读取 `serial.read` 返回 `text`；HEX 读取 `serial.read_hex` 返回 `hex`。两者共享块游标并报告实际字节数。
- 前端 Shell 和文本视图不调用二进制接口；仅 HEX 发送选项和 HEX 接收视图调用相应 HEX 接口。
- 串口插件移除 Base64 依赖及收发编码；Base64 是编码而非加密。基座认证和审批不变。
- Modbus、固件烧录等协议归独立插件，不在通用串口插件增加帧协议、CRC 或下载握手。

## 验证

普通测试覆盖控制字符、分段 UTF-8、无效 HEX、全部 256 种字节、参数混用拒绝、有界读取、审批及公共 MCP 名称路由；浏览器测试验证 Shell/UART 文本和 HEX 接口分离、复制粘贴、控制键与搜索。

本轮通过：`cargo test -p coterm-plugin-serial -p coterm-mcp`（46 项非硬件测试）、`cargo check --workspace`、串口全部 target 的 Clippy、`pnpm build`、`pnpm plugin:serial:build`，以及串口交互、串口/SSH 快捷键和桌面主题切换的浏览器测试。串口开发包已同步生成在 `plugins/plugin-serial/dist`。

本轮尝试 COM1/COM2 虚拟串口回环时，打开端口失败，系统返回“找不到指定的文件”。因此物理字节回环尚未验收；虚拟串口测试已更新，配置端口后可显式运行。

接口变化需同时重建串口插件后端和前端，旧版 `data`/Base64 请求不再兼容；MCP 客户端需重新发现工具。

## 2026-10-30 人工输入与 MCP 命令列表

- 人工 Shell 模式与 SSH 一样，在终端内直接输入；设备负责回显、行编辑和命令执行。
- 人工 UART 调试模式保留发送框，支持文本、HEX、行结束符与定时重复发送。
- 有序命令列表只用于 MCP 操作设备 Shell，减少 Agent 调用次数；人工终端不改为列表编辑器。
- 删除误加的 Shell 列表编辑器、间隔控件、队列按钮及相关前端解析、轮询和翻译。MCP 的 `serial.command` 后端队列、审批、取消、限额和设备完成语义保持原实现。
- MCP 进度事件仍更新 TX 计数；队列在途时输入隔离，完成后恢复人工输入。

验证：`pnpm build`、`pnpm plugin:serial:build`、串口终端浏览器回归、`pnpm test`（36 项）及 `cargo test -p coterm-plugin-serial`（26 单元、2 集成测试）通过。此轮未写入物理或虚拟设备，未修改后端/MCP/审批协议。

## 2026-10-30 COM1/COM2 虚拟配对验收

环境为 Windows x64、HHD Software Bridged Serial Port、COM1 ↔ COM2、115200 8-N-1、DTR/RTS 置高。测试启动真实宿主运行时和插件子进程，通过 OS 驱动双向收发；测试默认 ignored，必须显式配置端口。

```powershell
$env:COTERM_SERIAL_PAIR_A = "COM1"
$env:COTERM_SERIAL_PAIR_B = "COM2"
$env:COTERM_SERIAL_PAIR_BAUD = "115200"
cargo test -p coterm-plugin-serial --test virtual_pair -- --ignored --nocapture
```

COM1 → COM2 与交换方向均通过，退出后端口可重新打开。测试覆盖命令顺序/间隔、UTF-8 与 CRLF、反向二进制字节、取消、人工写入与信号冲突、MCP 审批、列表边界、队列容量、断开与重连回收。

发现 HHD 驱动的 `flush` 返回 Windows 错误码 1。写端现仅在驱动明确不支持 flush 时，于现有写入超时内查询发送队列并等待排空；其他错误、队列查询失败和排空超时均报错，不重发当前项或继续列表。

回归：插件 26 单元和 2 普通集成测试、workspace check、插件 Clippy、串口开发包构建通过；虚拟配对专项已单独执行。该链路验证真实宿主、插件 stdio 和虚拟驱动，不验证设备 Shell 执行、提示符、物理线路时序或硬件流控。

## 2026-10-30 真实串口硬件验收

设备为 COM6、115200 8-N-1、RISC-V Linux 板卡（PingDiChan，内核 5.4.61 riscv64），控制台登录 `root`/`root`。所有者连接并授权写入。

- 读取线程以停止标志和断开时 join 回收（上限 2 秒）；端口打开移入阻塞线程池，5 秒超时可生效。
- `serial.write` 标记 `sensitive_input`，MCP 镜像沿用标记；新增 `serial.signals` 操作提交/查询 DTR/RTS 电平。
- 接受 Unix `/dev/...` 设备节点，同时拒绝路径穿越与包外路径；此处仅在 Windows 验证名称规则。
- `serial.status` 提供 RX/TX 字节计数，TX 使用插件返回的实际字节数，含行结束符。
- 真实硬件验证登录交互、`uname -a` 往返、HEX 渲染、计数、DTR/RTS；12 轮连接/断开压力循环通过，销毁实例后端口可立即重开。
- 修复验收脚本语言假设和 TX 计数漏算行结束符问题。

验证：串口 Rust 测试 10 项、COM6 硬件用例、真实 Tauri + 板卡 UI 验收 8 项、Clippy/workspace 测试/`pnpm build`/`pnpm test` 通过。

未覆盖硬件流控、自动重连和热插拔；Unix 设备节点未在 Linux/macOS 实机运行；真实设备验证的是 Shell 回显，不是二进制文件传输。
