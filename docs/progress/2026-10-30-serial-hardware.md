# 2026-10-30 串口插件：真实硬件验收与 P0 收尾

设备：`COM6`，115200 8-N-1，RISC-V Linux 板卡（`PingDiChan`，内核 5.4.61 riscv64），
控制台登录 `root`/`root`。所有者已连接硬件并授权写入。

## 本轮完成

| 项 | 状态 |
|---|---|
| 读取线程停止与回收 | 读取循环抽出为 `read_until_stopped`；停止标志 + 断开时 join（上限 2 秒）；析构也置位标志 |
| 阻塞打开隔离 | 打开端口移入阻塞线程池，5 秒 `timeout` 可真正生效，超时后关闭端口并报错 |
| 写入脱敏 | `serial.write` 标记 `sensitive_input`；MCP 镜像沿用该标记，不能自报降级 |
| DTR/RTS | 配置项 `dtr`/`rts`，新操作 `serial.signals`（commit），状态回报当前电平 |
| Unix 端口 | 接受 `/dev/...` 设备节点，仍拒绝路径穿越与包外路径；平台相关的同名判断 |
| 收发计数 | `serial.status` 回报 `rxBytes`/`txBytes`；前端 TX 取插件回报的实际字节，含行结束符 |
| 前端重写 | 终端 + UART 调试台双视图、UTF-8/HEX 渲染、暂停/清空、重复发送、语言切换 |

## 验证

- `cargo test -p coterm-plugin-serial`：10 通过。
- `COTERM_SERIAL_LIVE=COM6` 的硬件用例：通过（字节往返、计数、信号）。
- `node scripts/test-serial-ui.mjs`（真实 Tauri + 真实板卡）：8 项全部通过，覆盖登录交互、
  `uname -a` 往返、HEX 视图、DTR/RTS、语言切换保持连接、断开收尾。
- 12 轮连接/断开压力循环零失败；销毁实例后端口可被新实例立即打开。
- `cargo clippy --workspace --all-targets -D warnings`、`cargo test --workspace`、`pnpm build`、`pnpm test` 通过。

## 过程中发现并修复

- 界面验收脚本原先按英文 `aria-label` 等待，中文系统下必然超时；改为用稳定的 `#locale-select` 判断并显式切英文。
- TX 计数只累加输入文本长度，漏掉插件补上的行结束符（`cr` 少 1 字节）；改取 `serial.write` 回报的 `bytes`，并在验收脚本里断言。

## 仍未覆盖

- 硬件流控（RTS/CTS 握手）、自动重连与热插拔监听。
- Unix 设备节点只在 Windows 上做了名字规则验证，未在 Linux/macOS 真机运行。
- 写入路径在真实设备上验证的是 shell 回显，未验证二进制文件传输。
