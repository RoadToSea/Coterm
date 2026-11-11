# 串口高波特率完整性未通过

## 复现环境与入口

- Windows 11、CH340／COM6、riscv64 Linux 开发板 `/dev/ttyS0`、Python 3.9.4，8N1，无软硬件流控。本机签名驱动为 wch.cn `3.9.2024.9`，日期为 2024-09-16；未升级或替换系统驱动。
- 以 115200 登录 Shell 后启动内存测速程序，切到 1500000 baud；每轮两个方向同时发送各 128 KiB，不同方向、轮次使用独立 SHA-256 文本负载。设备回传发送确认，主机逐字节校验接收内容。
- 显式执行以下命令，端口必须替换为实际已释放的测试设备：

```powershell
cargo build -p coterm-driver --release --example serial-diagnostic --locked
node scripts/benchmark-serial.mjs --port COM6 --baud 115200 --rates 1500000 --bytes 131072 --repeats 1 --seconds 600 --transfer-mode duplex --diagnostic target/release/examples/serial-diagnostic.exe --output .agent-debug/serial-throughput/report.json
```

## 已确认的观察

- 通知合并及 Windows overlapped IO 修复后，1500000 baud 同时收发仍可失败；最近一次第 26 轮接收失败，当前批次完整 BEGIN／END 边界内有 131,044 字节，预期 131,072，连续少 28 字节，首个差异在偏移 72,736。后端回放与实时通知一致，事件缺口为 0，不能把这个失败归为通知遗漏或旧历史淘汰。
- 降到 500000 baud 的最终全双工长测，前 96 轮每方向各 12,582,912 字节已核对，第 97 轮仍在完整边界内少 90 字节，首个差异在偏移 100,544。该阶段与本机 TCP 压测及工作区验证并行，通知缺口、板端 UART 错误计数均为 0；没有跑满计划的 600 秒，不能把短时通过当成长时间无损能力。
- 独立 Python 标准库 ctypes／Win32 overlapped 收发程序绕过 Coterm，保持相同设备、读写期限和负载，也在第 24 轮少 668 字节；该程序没有 Coterm 的缓冲、通知队列或调度，原生通信错误标志为 0。失败时板端 frame、overrun、parity、brk、bufferOverrun 均为 0，不能仅凭这些计数确定链路无损。
- 开发板 `tty.setraw()` 后仍保留 `IXOFF`，已捕获混入负载的 `0x13`。测速程序现在显式关闭 `IXON/IXOFF/IXANY` 并核验输入标志；关闭后仍存在上述连续缺字节，流控干扰不是所有失败的解释。
- 请求 64 KiB 系统收发队列曾在独立程序中通过 181 秒，但加入 Coterm 后第 2 轮仍少 256 字节。驱动在 4 KiB／64 KiB 请求下均报告 RX 队列 68,824、TX 队列 0；没有证据证明调整请求能修复问题，未保留这项修改。
- 失败用例均按非零退出并恢复原波特率及 Shell。不得将短测通过、中间原型的交替通过或无错误计数当作长期全双工无损认证。

## 待验证

- 缺失可在独立 Win32 路径复现，定位范围仍包括板端 UART／console、USB 转串口与连线、Windows USB／串口驱动及调度，尚未证明其中哪一项是根因。
- 下一步优先更换适配器或在另一系统上使用相同负载交叉对照，不继续以扩大 Coterm 通知队列、暂停读取或过滤收到的字节来掩盖失败。
- 较低速率验收与本轮软件修复的最终结果见 [Serial 进度](../../crates/coterm-driver/docs/serial/progress.md#2026-11-11系统读取与通知链路修复)。已验证的范围不等于硬件或产品的通用速率上限。
