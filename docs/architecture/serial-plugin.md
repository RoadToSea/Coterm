# 串口插件

实现位于 [plugin-serial](../../plugins/plugin-serial)。插件只处理原始字节，不解析设备提示符、不推断命令完成或退出码。

## 使用

按 [README](../../README.md) 安装依赖后，执行 `pnpm plugin:serial:build` 和 `pnpm tauri:dev`。

填写 Port、Baud rate、Data bits、Stop bits、Parity、DTR、RTS 创建实例，默认 115200 8-N-1、DTR 与 RTS 置高；在实例面板 Connect。配置阶段手填端口，创建实例后可用 Ports 枚举。实例切换不关闭连接，同一端口不能被多个实例占用。

端口名按平台规则校验：Windows 接受 `COM6` 这类设备名（不区分大小写），Unix 接受 `/dev/...` 设备节点；其余带分隔符的名字一律拒绝，避免把配置变成任意文件打开入口。部分开发板会因 DTR 变化复位，可为这类板子取消「打开时置高 DTR」。

终端按键发送原始字节；发送框支持 HEX 与 none/lf/cr/crlf 行结束符。写入总量为 1～16384 字节，HEX 必须为偶数位有效十六进制字符。

## 操作与 MCP

| 操作 | effect | 用途 |
|---|---|---|
| `serial.status/probe` | read | 状态与系统端口枚举 |
| `serial.connect/disconnect` | commit | 打开或关闭端口，清理该实例旧提案 |
| `serial.read` | read | 游标回放 base64 字节，报告 gap/closed/more |
| `serial.write` | commit | 人工字节写入；标记为敏感输入 |
| `serial.signals` | commit | 设置 DTR/RTS 电平 |
| `serial.plugin_info` | read | 插件级信息 |

MCP 提供上述工具中的 status、probe、connect、disconnect、read、plugin_info，**不开放 write 与 signals**。除 plugin_info 外必须明确实例 ID。取消请求目前返回 unsupported，不冒充设备已停止。

## 数据与已知限制

- 每连接缓存 256 KiB，每块及单次读取最多 8 KiB；游标过期报告 gap，超出末尾报错。
- `serial.output`/`serial.status` 使用连接 ID 和游标；通知队列有界，推送丢失后以回放缓冲恢复。
- 读取使用独立线程，写入经 64 槽队列和阻塞任务提交；前端输入队列上限 16 KiB。
- 读取线程有停止标志，显式断开时等待其退出并回收句柄（上限 2 秒）；析构也会置位该标志，避免线程泄漏。停止标志只在一次阻塞读之后检查，最坏退出延迟是一次读取轮询（20 ms）。
- 打开端口是阻塞调用，放在阻塞线程池执行；5 秒超时后关闭端口并报错，不占用异步工作线程。
- `serial.status` 回报 DTR/RTS 电平与本次连接累计收发字节数；前端 TX 计数取插件回报的实际写入字节，包含插件补上的行结束符。
- `serial.write` 标记为敏感输入，人工输入可能是密码或密钥，账本只保留脱敏占位。
- 尚无硬件流控（RTS/CTS 握手）、自动重连、热插拔监听和文件收发。

公共交互、事件和账本规则见 [运行时通信](runtime-communication.md)，修复优先级见 [项目进度](../PROGRESS.md)。

## 验证

普通契约与基座加载测试不打开或写入物理端口：

```bash
cargo test -p coterm-plugin-serial
```

硬件用例默认 ignored。确认目标设备可接收写入后显式执行，将示例端口替换为实际端口：

```powershell
$env:COTERM_SERIAL_LIVE = "COM6"
$env:COTERM_SERIAL_LIVE_BAUD = "115200"
cargo test -p coterm-plugin-serial --test live -- --ignored --nocapture
```

该用例验证真实字节往返、收发计数与 DTR/RTS 落地，不验证设备业务语义；缺少设备配置会失败。手工诊断可用 `cargo run -p coterm-plugin-serial --example live -- COM6 115200`，不属于普通回归。

界面验收在真实 Tauri 中运行，需要可枚举的串口（默认 `COM6`，可用 `COTERM_SERIAL_LIVE_PORT` 覆盖）：

```bash
pnpm plugin:serial:build
pnpm --filter coterm-desktop dev   # 另一个终端保持运行
node scripts/test-serial-ui.mjs
```

脚本会连接开发板并按需登录（`COTERM_SERIAL_USER`/`COTERM_SERIAL_PASSWORD`，默认 `root`/`root`），随后验证终端与 UART 两种视图、UTF-8 与 HEX 渲染、收发计数、DTR/RTS 切换、语言切换保持连接，以及断开后的干净状态。它会向物理设备写入字节。
