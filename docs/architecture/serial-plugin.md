# 串口插件

实现位于 [plugin-serial](../../plugins/plugin-serial)。插件只处理原始字节，不解析设备提示符、不推断命令完成或退出码。

## 使用

按 [README](../../README.md) 安装依赖后，执行 `pnpm plugin:serial:build` 和 `pnpm tauri:dev`。

填写 Port、Baud rate、Data bits、Stop bits、Parity 创建实例，默认 115200 8-N-1；在实例面板 Connect。配置阶段手填端口，创建实例后可用 Ports 枚举。实例切换不关闭连接，同名端口不能被多个实例占用。

终端按键发送原始字节；发送框支持 HEX 与 none/lf/cr/crlf 行结束符。写入总量为 1～16384 字节，HEX 必须为偶数位有效十六进制字符。

## 操作与 MCP

| 操作 | effect | 用途 |
|---|---|---|
| `serial.status/probe` | read | 状态与系统端口枚举 |
| `serial.connect/disconnect` | commit | 打开或关闭端口，清理该实例旧提案 |
| `serial.read` | read | 游标回放 base64 字节，报告 gap/closed/more |
| `serial.write` | commit | 人工字节写入 |
| `serial.plugin_info` | read | 插件级信息 |

MCP 提供上述工具中的 status、probe、connect、disconnect、read、plugin_info，**不开放 write**。除 plugin_info 外必须明确实例 ID。取消请求目前返回 unsupported，不冒充设备已停止。

## 数据与已知限制

- 每连接缓存 256 KiB，每块及单次读取最多 8 KiB；游标过期报告 gap，超出末尾报错。
- `serial.output`/`serial.status` 使用连接 ID 和游标；通知队列有界，推送丢失后以回放缓冲恢复。
- 读取使用独立线程，写入经 64 槽队列和阻塞任务提交；前端输入队列上限 16 KiB。
- **待修复**：读取线程缺少停止/回收；同步打开端口未隔离，timeout 不能中断它；原始输入尚未标记敏感参数。
- **跨平台缺口**：配置拒绝包含斜杠的端口名，包括 Unix `/dev/...`；当前不能按 manifest 声明认定 Linux/macOS 已可用。
- 尚无流控、DTR/RTS 控制、自动重连、热插拔监听和文件收发。

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

该用例验证真实字节往返，不验证设备业务语义；缺少设备配置会失败。手工诊断可用 `cargo run -p coterm-plugin-serial --example live -- COM6 115200`，不属于普通回归。
